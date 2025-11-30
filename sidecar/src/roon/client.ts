/**
 * Roon API client with auto-discovery and connection management
 *
 * Handles:
 * - Roon Core discovery
 * - Pairing/authorization
 * - Connection state management
 * - Service initialization
 */

import RoonApi from 'node-roon-api';
import RoonApiTransport from 'node-roon-api-transport';
import RoonApiImage from 'node-roon-api-image';
import * as output from '../output.js';
import { TransportManager } from './transport.js';
import { ImageManager } from './image.js';
import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';

interface RoonCore {
  display_name: string;
  display_version: string;
  services: {
    RoonApiTransport?: any;
    RoonApiImage?: any;
  };
}

/**
 * Extended RoonApi options interface with all callbacks
 * The node-roon-api package doesn't have complete TypeScript types
 */
interface ExtendedRoonApiOptions {
  extension_id: string;
  display_name: string;
  display_version: string;
  publisher: string;
  email: string;
  website: string;
  core_found?: (core: RoonCore) => void;
  core_lost?: (core: RoonCore) => void;
  core_paired?: (core: RoonCore) => void;
  core_unpaired?: (core: RoonCore) => void;
  log_level?: 'all' | 'none';
  set_persisted_state?: (state: any) => void;
  get_persisted_state?: () => any;
}

/**
 * Get the config directory path for storing pairing data
 */
function getConfigPath(): string {
  const homeDir = os.homedir();
  const configDir = path.join(homeDir, '.config', 'now-playing');

  // Ensure directory exists
  if (!fs.existsSync(configDir)) {
    fs.mkdirSync(configDir, { recursive: true });
  }

  return path.join(configDir, 'roon-config.json');
}

/**
 * Main Roon client class
 */
export class RoonClient {
  private roonApi: any;
  private imageManager: ImageManager;
  private transportManager: TransportManager;
  private isAuthorized: boolean = false;
  private currentCore: RoonCore | null = null;

  constructor() {
    this.imageManager = new ImageManager();
    this.transportManager = new TransportManager(this.imageManager);
    this.roonApi = this.createRoonApi();
  }

  /**
   * Create and configure the Roon API instance
   */
  private createRoonApi(): any {
    // Initialize Roon API with extension information
    const hostname = os.hostname();
    const options: ExtendedRoonApiOptions = {
      extension_id: 'com.nowplaying.menubar',
      display_name: `Now Playing Menu Bar (${hostname})`,
      display_version: '0.1.0',
      publisher: 'Now Playing',
      email: 'REDACTED_EMAIL',
      website: 'REDACTED_WEBSITE',

      // IMPORTANT: Roon API does NOT allow both core_found AND core_paired
      // We use core_paired/core_unpaired for automatic pairing management
      // core_found/core_lost are for manual pairing control (mutually exclusive)

      core_paired: (core: RoonCore) => {
        output.info('=== CORE PAIRED CALLBACK TRIGGERED ===');
        output.info(`Core: ${core.display_name} ${core.display_version}`);
        output.info(`Core services available: ${JSON.stringify(Object.keys(core.services || {}))}`);
        this.handleCorePaired(core);
      },

      core_unpaired: (core: RoonCore) => {
        output.info('=== CORE UNPAIRED CALLBACK TRIGGERED ===');
        output.info(`Core: ${core.display_name}`);
        this.handleCoreUnpaired(core);
      },

      // Log level (can be set to 'all' for debugging)
      log_level: 'all',

      // Pairing persistence - saves auth tokens for automatic reconnection
      set_persisted_state: (state: any) => {
        try {
          const configPath = getConfigPath();
          fs.writeFileSync(configPath, JSON.stringify(state, null, 2));
          output.debug(`Saved pairing state to ${configPath}`);
        } catch (error) {
          output.error('Failed to save pairing state:', error);
        }
      },

      get_persisted_state: () => {
        try {
          const configPath = getConfigPath();
          if (fs.existsSync(configPath)) {
            const data = fs.readFileSync(configPath, 'utf8');
            output.debug(`Loaded pairing state from ${configPath}`);
            const state = JSON.parse(data);
            // Ensure tokens object exists
            if (!state.tokens) {
              state.tokens = {};
            }
            return state;
          }
        } catch (error) {
          output.error('Failed to load pairing state:', error);
        }
        // CRITICAL: Must return an object with tokens property, not null
        // The Roon API expects this structure even for first-time pairing
        output.debug('No persisted state found, returning empty state with tokens object');
        return { tokens: {} };
      },
    };

    output.info('Creating RoonApi instance with callbacks...');
    const roon = new RoonApi(options);

    output.info('Initializing Roon services...');
    // Initialize services - these must be set up before start_discovery/ws_connect
    // Using required_services means the extension needs these to function
    roon.init_services({
      provided_services: [],
      required_services: [RoonApiTransport, RoonApiImage],
    });

    output.info('RoonApi instance created successfully');
    output.info(`Callbacks registered: core_found=${!!options.core_found}, core_paired=${!!options.core_paired}`);

    return roon;
  }

  /**
   * Handle core pairing (authorization granted or reconnected with saved credentials)
   */
  private handleCorePaired(core: RoonCore): void {
    output.info(`Core paired: ${core.display_name} ${core.display_version}`);
    this.isAuthorized = true;
    this.currentCore = core;

    // Emit connected status
    output.emitStatus('connected', `Connected to ${core.display_name}`);

    // Initialize services
    this.initializeServices(core);
  }

  /**
   * Handle core unpairing (connection lost or unpaired)
   */
  private handleCoreUnpaired(core: RoonCore): void {
    output.info(`Core unpaired: ${core.display_name}`);
    this.isAuthorized = false;
    this.currentCore = null;

    // Emit disconnected status
    output.emitStatus('disconnected', 'Disconnected from Roon Core');

    // Clear services
    this.transportManager.clearTransportService();
    this.imageManager.clearImageService();

    // Emit stopped state (no zone_id since disconnected)
    output.emitNowPlaying('', '', '', '', 'stopped');
  }

  /**
   * Initialize Roon services after pairing
   */
  private initializeServices(core: RoonCore): void {
    try {
      output.info('=== INITIALIZING SERVICES ===');
      output.info(`Available services: ${JSON.stringify(Object.keys(core.services || {}))}`);

      // Get transport service
      const transportService = core.services.RoonApiTransport;
      if (transportService) {
        output.info('Transport service found, initializing...');
        this.transportManager.setTransportService(transportService);
        output.info('✓ Transport service initialized successfully');
      } else {
        output.warn('✗ Transport service not available in core.services');
        output.warn(`Core services object: ${JSON.stringify(core.services)}`);
      }

      // Get image service
      const imageService = core.services.RoonApiImage;
      if (imageService) {
        output.info('Image service found, initializing...');
        this.imageManager.setImageService(imageService);
        output.info('✓ Image service initialized successfully');
      } else {
        output.warn('✗ Image service not available in core.services');
      }

      output.info('=== SERVICE INITIALIZATION COMPLETE ===');
    } catch (error) {
      output.error('Failed to initialize services:', error);
      output.emitError('Failed to initialize Roon services');
    }
  }

  /**
   * Start the Roon client and begin discovery or connect to a specific host
   */
  start(): void {
    output.info('=== STARTING ROON CLIENT ===');
    output.info(`RoonApi instance exists: ${!!this.roonApi}`);
    output.info(`RoonApi methods available: ${Object.keys(this.roonApi || {}).join(', ')}`);

    // Check for manual host configuration via environment variable
    const roonHost = process.env.ROON_HOST;
    const roonPort = process.env.ROON_PORT ? parseInt(process.env.ROON_PORT) : 9100;

    try {
      if (roonHost) {
        // Manual connection to specific host
        output.info(`=== USING DIRECT CONNECTION MODE ===`);
        output.info(`Connecting directly to Roon Core at ${roonHost}:${roonPort}`);
        output.emitStatus('discovering', `Connecting to ${roonHost}...`);

        try {
          output.info('Calling roonApi.ws_connect()...');
          this.roonApi.ws_connect({
            host: roonHost,
            port: roonPort,
            onclose: () => {
              output.warn('WebSocket connection to Roon Core closed');
              output.emitStatus('disconnected', 'Connection to Roon Core lost');

              // Try to reconnect after a delay
              setTimeout(() => {
                output.info('Attempting to reconnect...');
                this.start();
              }, 5000);
            }
          });

          output.info(`✓ WebSocket connection initiated to ${roonHost}:${roonPort}`);
          output.info('Waiting for core_found and core_paired callbacks...');
        } catch (err) {
          output.error('Error calling ws_connect:', err);
          output.emitError('Failed to connect to Roon Core: ' + (err instanceof Error ? err.message : String(err)));
          throw err;
        }
      } else {
        // Auto-discovery
        output.info('=== USING AUTO-DISCOVERY MODE ===');
        output.emitStatus('discovering', 'Searching for Roon Core...');

        output.info('Calling roonApi.start_discovery()...');
        this.roonApi.start_discovery();
        output.info('✓ Roon discovery started');
        output.info('Waiting for core_found and core_paired callbacks...');
      }

      // Set up periodic connection status checks
      this.startConnectionMonitor();

      output.info('=== ROON CLIENT START COMPLETE ===');
    } catch (error) {
      output.error('Failed to start Roon client:', error);
      output.emitError(
        error instanceof Error ? error.message : 'Failed to start Roon client'
      );
      throw error;
    }
  }

  /**
   * Monitor connection status periodically
   */
  private startConnectionMonitor(): void {
    // Check connection status every 30 seconds
    setInterval(() => {
      if (!this.isAuthorized && !this.currentCore) {
        // Still discovering
        output.debug('Still searching for Roon Core...');
      }
    }, 30000);
  }

  /**
   * Stop the Roon client
   */
  stop(): void {
    output.info('Stopping Roon client...');

    try {
      // Clean up services
      this.transportManager.clearTransportService();
      this.imageManager.clearImageService();

      // Stop discovery (if the API supports it)
      // Note: node-roon-api doesn't have an explicit stop method,
      // but cleaning up will happen when the process exits

      output.info('Roon client stopped');
    } catch (error) {
      output.error('Error stopping Roon client:', error);
    }
  }
/**
 * Get authorization status
 */
isConnected(): boolean {
  return this.isAuthorized && this.currentCore !== null;
}

/**
 * Get current core information
 */
getCurrentCore(): RoonCore | null {
  return this.currentCore;
}
}


