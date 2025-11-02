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

interface RoonCore {
  display_name: string;
  display_version: string;
  services: {
    RoonApiTransport?: any;
    RoonApiImage?: any;
  };
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
    const roon = new RoonApi({
      extension_id: 'com.nowplaying.menubar',
      display_name: 'Now Playing Menu Bar',
      display_version: '0.1.0',
      publisher: 'Now Playing',
      email: 'REDACTED_EMAIL',
      website: 'REDACTED_WEBSITE',

      // Set up core pairing callbacks
      core_paired: (core: RoonCore) => {
        output.info('*** CORE PAIRED CALLBACK TRIGGERED ***');
        this.handleCorePaired(core);
      },

      core_unpaired: (core: RoonCore) => {
        output.info('*** CORE UNPAIRED CALLBACK TRIGGERED ***');
        this.handleCoreUnpaired(core);
      },

      // Log level (can be set to 'all' for debugging)
      log_level: 'all',
    });

    // Initialize services
    // Note: Using provided_services means we provide functionality TO Roon
    // We actually want to consume services, but we'll make them "optional" so
    // the extension shows up before authorization
    roon.init_services({
      provided_services: [],
      required_services: [RoonApiTransport, RoonApiImage],
    });

    return roon;
  }

  /**
   * Handle core pairing (authorization granted)
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

    // Emit stopped state
    output.emitNowPlaying('', '', '', 'stopped');
  }

  /**
   * Initialize Roon services after pairing
   */
  private initializeServices(core: RoonCore): void {
    try {
      // Get transport service
      const transportService = core.services.RoonApiTransport;
      if (transportService) {
        this.transportManager.setTransportService(transportService);
        output.debug('Transport service initialized');
      } else {
        output.warn('Transport service not available');
      }

      // Get image service
      const imageService = core.services.RoonApiImage;
      if (imageService) {
        this.imageManager.setImageService(imageService);
        output.debug('Image service initialized');
      } else {
        output.warn('Image service not available');
      }
    } catch (error) {
      output.error('Failed to initialize services:', error);
      output.emitError('Failed to initialize Roon services');
    }
  }

  /**
   * Start the Roon client and begin discovery or connect to a specific host
   */
  start(): void {
    output.info('Starting Roon client...');

    // Check for manual host configuration via environment variable
    const roonHost = process.env.ROON_HOST;
    const roonPort = process.env.ROON_PORT ? parseInt(process.env.ROON_PORT) : 9100;

    try {
      if (roonHost) {
        // Manual connection to specific host
        output.info(`Connecting directly to Roon Core at ${roonHost}:${roonPort}`);
        output.emitStatus('discovering', `Connecting to ${roonHost}...`);

        try {
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

          output.info(`WebSocket connection initiated to ${roonHost}:${roonPort}`);
        } catch (err) {
          output.error('Error calling ws_connect:', err);
          output.emitError('Failed to connect to Roon Core: ' + (err instanceof Error ? err.message : String(err)));
          throw err;
        }
      } else {
        // Auto-discovery
        output.emitStatus('discovering', 'Searching for Roon Core...');
        this.roonApi.start_discovery();
        output.info('Roon discovery started');
      }

      // Set up periodic connection status checks
      this.startConnectionMonitor();
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
