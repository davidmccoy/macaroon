/**
 * Transport module for zone subscriptions and now playing data extraction
 *
 * Handles subscribing to Roon zones and extracting now playing information
 */

import * as output from '../output.js';
import { ImageManager } from './image.js';

export interface TransportService {
  subscribe_zones: (callback: (response: string, data: any) => void) => void;
  subscribe_outputs: (callback: (response: string, data: any) => void) => void;
}

interface Zone {
  zone_id: string;
  display_name: string;
  outputs: any[];
  now_playing?: NowPlayingData;
  state?: string;
}

interface Output {
  output_id: string;
  zone_id?: string;
  display_name: string;
  state?: string;
  source_controls?: Array<{
    control_key: string;
    display_name: string;
    status: string; // 'selected', 'standby', 'deselected', etc.
    supports_standby: boolean;
  }>;
}

interface NowPlayingData {
  seek_position?: number;
  length?: number;
  image_key?: string;
  one_line?: {
    line1: string;
  };
  two_line?: {
    line1: string;
    line2: string;
  };
  three_line?: {
    line1: string;
    line2: string;
    line3: string;
  };
}

interface ZonesData {
  zones?: Zone[];
  zones_changed?: Zone[];
  zones_removed?: string[];
  zones_seek_changed?: any[];
}

interface OutputsData {
  outputs?: Output[];
  outputs_changed?: Output[];
  outputs_removed?: string[];
}

/**
 * Manages zone subscriptions and now playing state
 */
export class TransportManager {
  private transportService: TransportService | null = null;
  private imageManager: ImageManager;
  private allZones: Map<string, Zone> = new Map(); // Track ALL zones by zone_id
  private allOutputs: Map<string, Output> = new Map(); // Track ALL outputs by output_id

  constructor(imageManager: ImageManager) {
    this.imageManager = imageManager;
  }

  /**
   * Set the Roon transport service and start subscribing to zones
   */
  setTransportService(service: TransportService): void {
    output.info('=== SETTING TRANSPORT SERVICE ===');
    output.info(`Service object received: ${!!service}`);
    output.info(`Service has subscribe_zones: ${!!(service && service.subscribe_zones)}`);
    output.info(`Service has subscribe_outputs: ${!!(service && service.subscribe_outputs)}`);

    this.transportService = service;
    output.info('Transport service stored, now subscribing to zones and outputs...');
    this.subscribeToZones();
    this.subscribeToOutputs();
  }

  /**
   * Clear the transport service
   */
  clearTransportService(): void {
    this.transportService = null;
    this.allZones.clear();
    this.allOutputs.clear();
    output.debug('Transport service cleared');
  }

  /**
   * Emit the current zone list to Rust
   * Includes both active zones and standby outputs (which aren't in zones yet)
   */
  private emitCurrentZoneList(): void {
    // Collect zone IDs that have active zones
    const zoneOutputIds = new Set<string>();
    this.allZones.forEach(zone => {
      zone.outputs?.forEach(out => {
        if (out.output_id) {
          zoneOutputIds.add(out.output_id);
        }
      });
    });

    // Build zone list from active zones
    const zones: output.ZoneInfo[] = Array.from(this.allZones.values()).map(zone => {
      const state = this.mapRoonStateToPlaybackState(zone.state || 'stopped');
      const zoneInfo: output.ZoneInfo = {
        zone_id: zone.zone_id,
        display_name: zone.display_name,
        state,
      };

      // Include now_playing if available
      if (zone.now_playing && (state === 'playing' || state === 'paused')) {
        const { title, artist, album } = this.extractMetadata(zone.now_playing);
        zoneInfo.now_playing = {
          title,
          artist,
          album,
          // Note: We don't include artwork in zone_list to keep it lightweight
          // Artwork is only included in now_playing messages
        };
      }

      return zoneInfo;
    });

    // Add outputs that aren't part of any active zone
    // These are outputs that exist but aren't currently in the zones list
    this.allOutputs.forEach(out => {
      // Skip if this output is already represented in a zone
      if (zoneOutputIds.has(out.output_id)) {
        return;
      }

      // Include any output that's not in an active zone
      // Status can be 'standby', 'selected', 'indeterminate', etc.
      zones.push({
        zone_id: `output:${out.output_id}`, // Prefix to distinguish from real zones
        display_name: `${out.display_name} (Inactive)`,
        state: 'stopped',
      });
      output.debug(`Including inactive output: ${out.display_name}`);
    });

    if (zones.length > 0) {
      output.emitZoneList(zones);
      output.debug(`Emitted zone list with ${zones.length} zone(s)/output(s)`);
    }
  }

  /**
   * Map Roon state to our PlaybackState type
   */
  private mapRoonStateToPlaybackState(state: string): output.PlaybackState {
    switch (state) {
      case 'playing':
        return 'playing';
      case 'paused':
        return 'paused';
      case 'loading':
        return 'loading';
      default:
        return 'stopped';
    }
  }

  /**
   * Subscribe to all zones and listen for updates
   */
  private subscribeToZones(): void {
    if (!this.transportService) {
      output.warn('Cannot subscribe to zones: transport service not available');
      return;
    }

    output.info('=== SUBSCRIBING TO ZONES ===');
    output.info('Calling transportService.subscribe_zones()...');

    try {
      this.transportService.subscribe_zones((response: string, data: ZonesData) => {
        output.info(`=== ZONE SUBSCRIPTION CALLBACK FIRED ===`);
        output.info(`Response type: ${response}`);
        output.info(`Data keys: ${JSON.stringify(Object.keys(data || {}))}`);

        if (response === 'Subscribed') {
          output.info('✓ Successfully subscribed to zones');
          output.info(`Initial zones count: ${(data.zones || []).length}`);
          // Initial zone data
          this.handleZonesUpdate(data);
        } else if (response === 'Changed') {
          output.info('Zone state changed');
          output.info(`Changed zones count: ${(data.zones_changed || []).length}`);
          // Zone state changed
          this.handleZonesUpdate(data);
        } else if (response === 'NetworkError' || response === 'ConnectionError') {
          output.warn(`Connection error: ${response}`);
          output.emitStatus('disconnected', 'Lost connection to Roon Core');
        } else {
          output.warn(`Unknown zone subscription response: ${response}`);
        }
      });

      output.info('✓ Zone subscription callback registered');
    } catch (error) {
      output.error('Error subscribing to zones:', error);
      output.emitError('Failed to subscribe to zones');
    }
  }

  /**
   * Subscribe to all outputs and listen for updates
   * Outputs include standby devices that may not be in active zones
   */
  private subscribeToOutputs(): void {
    if (!this.transportService) {
      output.warn('Cannot subscribe to outputs: transport service not available');
      return;
    }

    output.info('=== SUBSCRIBING TO OUTPUTS ===');
    output.info('Calling transportService.subscribe_outputs()...');

    try {
      this.transportService.subscribe_outputs((response: string, data: OutputsData) => {
        output.debug(`Output subscription callback: ${response}`);

        if (response === 'Subscribed') {
          output.info('✓ Successfully subscribed to outputs');
          output.info(`Initial outputs count: ${(data.outputs || []).length}`);
          this.handleOutputsUpdate(data);
        } else if (response === 'Changed') {
          output.debug(`Outputs changed: ${(data.outputs_changed || []).length}`);
          this.handleOutputsUpdate(data);
        } else if (response === 'NetworkError' || response === 'ConnectionError') {
          output.warn(`Output subscription error: ${response}`);
        } else {
          output.debug(`Unknown output subscription response: ${response}`);
        }
      });

      output.info('✓ Output subscription callback registered');
    } catch (error) {
      output.error('Error subscribing to outputs:', error);
      // Don't emit error to Rust - outputs are supplementary
    }
  }

  /**
   * Handle output updates from Roon
   */
  private handleOutputsUpdate(data: OutputsData): void {
    // Handle outputs_removed
    if (data.outputs_removed && data.outputs_removed.length > 0) {
      data.outputs_removed.forEach(outputId => {
        if (this.allOutputs.has(outputId)) {
          this.allOutputs.delete(outputId);
          output.info(`Removed output: ${outputId}`);
        }
      });
    }

    // Handle new or changed outputs
    const outputs = data.outputs || data.outputs_changed || [];
    outputs.forEach(out => {
      const existing = this.allOutputs.get(out.output_id);
      if (!existing) {
        output.info(`New output: ${out.display_name} (${out.output_id})`);
      }
      // Always update the output data
      this.allOutputs.set(out.output_id, out);
    });

    // Always emit zone list when we receive output updates
    // This ensures inactive outputs are always reflected in the menu
    this.emitCurrentZoneList();
  }

  /**
   * Handle zone updates from Roon
   */
  private async handleZonesUpdate(data: ZonesData): Promise<void> {
    try {
      output.info('=== HANDLING ZONES UPDATE ===');

      // Handle seek position changes separately - these don't contain full zone data
      if (data.zones_seek_changed && data.zones_seek_changed.length > 0 && !data.zones && !data.zones_changed) {
        output.debug(`Seek position update for ${data.zones_seek_changed.length} zone(s) - ignoring (no zone data)`);
        return;
      }

      // Handle zones_removed
      if (data.zones_removed && data.zones_removed.length > 0) {
        output.info(`Removing ${data.zones_removed.length} zone(s)`);
        data.zones_removed.forEach(zoneId => {
          this.allZones.delete(zoneId);
          output.info(`Removed zone: ${zoneId}`);
        });
        // Emit updated zone list immediately
        this.emitCurrentZoneList();
      }

      // Get all zones (either from initial subscription or changes)
      const zones = data.zones || data.zones_changed || [];

      output.info(`Total zones received: ${zones.length}`);

      if (zones.length === 0) {
        output.debug('No zones in update');
        return;
      }

      // Update our zone map with all received zones
      zones.forEach(zone => {
        this.allZones.set(zone.zone_id, zone);
        output.info(`Zone updated: ${zone.display_name} (${zone.zone_id}) - state: ${zone.state}`);
      });

      // Emit updated zone list immediately when zones change
      this.emitCurrentZoneList();

      // Emit now_playing for ALL playing/paused zones so artwork is available
      // when the user switches between zones
      const playingZones = zones.filter(
        (zone) => zone.state === 'playing' || zone.state === 'paused'
      );

      for (const zone of playingZones) {
        output.debug(`Emitting now_playing for zone: ${zone.display_name} (${zone.state})`);
        await this.extractAndEmitNowPlaying(zone);
      }
    } catch (error) {
      output.error('Error handling zone update:', error);
    }
  }

  /**
   * Extract metadata from now_playing data
   */
  private extractMetadata(nowPlaying: NowPlayingData): { title: string; artist: string; album: string } {
    let title = '';
    let artist = '';
    let album = '';

    if (nowPlaying.three_line) {
      // three_line typically has: line1=title, line2=artist, line3=album
      title = nowPlaying.three_line.line1 || '';
      artist = nowPlaying.three_line.line2 || '';
      album = nowPlaying.three_line.line3 || '';
    } else if (nowPlaying.two_line) {
      // two_line typically has: line1=title, line2=artist
      title = nowPlaying.two_line.line1 || '';
      artist = nowPlaying.two_line.line2 || '';
    } else if (nowPlaying.one_line) {
      // one_line just has the title
      title = nowPlaying.one_line.line1 || '';
    }

    return { title, artist, album };
  }

  /**
   * Extract now playing information from a zone and emit it
   */
  private async extractAndEmitNowPlaying(zone: Zone): Promise<void> {
    const nowPlaying = zone.now_playing;
    const state = zone.state || 'stopped';

    // Handle stopped state
    if (state === 'stopped' || !nowPlaying) {
      output.emitNowPlaying(zone.zone_id, '', '', '', 'stopped');
      return;
    }

    // Extract metadata
    const { title, artist, album } = this.extractMetadata(nowPlaying);

    // Map Roon state to our state enum
    const playbackState = this.mapRoonStateToPlaybackState(state);

    // Fetch artwork if available (image manager has caching)
    let artwork: string | undefined;
    const imageKey = nowPlaying.image_key;

    if (imageKey) {
      artwork = await this.imageManager.fetchArtwork(imageKey);
    }

    // Emit the now playing data with zone_id
    output.emitNowPlaying(zone.zone_id, title, artist, album, playbackState, artwork);
    output.debug(`Emitted now playing for zone ${zone.zone_id}: ${title} by ${artist} (${playbackState})`);
  }
}
