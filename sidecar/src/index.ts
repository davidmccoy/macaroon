#!/usr/bin/env node

/**
 * Now Playing Sidecar - Roon API Integration
 *
 * This sidecar process connects to Roon Core, subscribes to now playing updates,
 * and emits JSON messages to stdout for consumption by the main Rust application.
 *
 * Communication protocol:
 * - stdout: JSON messages (one per line)
 * - stderr: Debug/log messages
 */

import { RoonClient } from './roon/client.js';
import * as output from './output.js';

/**
 * Main entry point
 */
function main(): void {
  output.info('=== Now Playing Sidecar Starting ===');
  output.info(`Node version: ${process.version}`);
  output.info(`Platform: ${process.platform} ${process.arch}`);

  // Create and start Roon client
  const client = new RoonClient();

  try {
    client.start();
    output.info('Sidecar running, waiting for Roon Core...');
  } catch (error) {
    output.error('Fatal error starting sidecar:', error);
    output.emitError(
      error instanceof Error ? error.message : 'Unknown error starting sidecar'
    );
    process.exit(1);
  }

  // Handle graceful shutdown
  const shutdown = () => {
    output.info('Shutting down sidecar...');
    client.stop();
    process.exit(0);
  };

  // Listen for shutdown signals
  process.on('SIGINT', shutdown);
  process.on('SIGTERM', shutdown);

  // Handle uncaught errors
  process.on('uncaughtException', (error) => {
    output.error('Uncaught exception:', error);
    output.emitError(`Uncaught exception: ${error.message}`);
    // Don't exit immediately, let the client try to recover
  });

  process.on('unhandledRejection', (reason, promise) => {
    output.error('Unhandled rejection at:', promise, 'reason:', reason);
    output.emitError(`Unhandled rejection: ${reason}`);
    // Don't exit immediately, let the client try to recover
  });

  // Keep the process alive
  // The Roon client will handle all events asynchronously
}

// Start the sidecar
main();
