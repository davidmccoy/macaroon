/**
 * Output module for emitting JSON messages to stdout
 *
 * These messages are consumed by the Rust main application via stdin/stdout IPC.
 * Each message must be a single line of JSON.
 */

export type PlaybackState = 'playing' | 'paused' | 'stopped' | 'loading';

export type ConnectionState = 'discovering' | 'not_authorized' | 'connected' | 'disconnected';

export interface NowPlayingOutput {
  type: 'now_playing';
  zone_id: string; // NEW: Zone identifier
  title: string;
  artist: string;
  album: string;
  state: PlaybackState;
  artwork?: string; // base64 data URL
}

export interface ZoneInfo {
  zone_id: string;
  display_name: string;
  state: PlaybackState;
  now_playing?: {
    title: string;
    artist: string;
    album: string;
    artwork?: string;
  };
}

export interface ZoneListOutput {
  type: 'zone_list';
  zones: ZoneInfo[];
}

export interface StatusOutput {
  type: 'status';
  state: ConnectionState;
  message?: string;
}

export interface ErrorOutput {
  type: 'error';
  message: string;
}

export type SidecarOutput = NowPlayingOutput | ZoneListOutput | StatusOutput | ErrorOutput;

/**
 * Emit a JSON message to stdout
 * Each message is a complete JSON object on a single line
 */
export function emit(data: SidecarOutput): void {
  try {
    const json = JSON.stringify(data);
    console.log(json);
  } catch (error) {
    // Fallback error output if JSON serialization fails
    console.error(JSON.stringify({
      type: 'error',
      message: error instanceof Error ? error.message : 'Unknown error in JSON serialization',
    }));
  }
}

/**
 * Emit a now playing update
 */
export function emitNowPlaying(
  zone_id: string,
  title: string,
  artist: string,
  album: string,
  state: PlaybackState,
  artwork?: string
): void {
  emit({
    type: 'now_playing',
    zone_id,
    title,
    artist,
    album,
    state,
    artwork,
  });
}

/**
 * Emit a zone list update
 */
export function emitZoneList(zones: ZoneInfo[]): void {
  emit({
    type: 'zone_list',
    zones,
  });
}

/**
 * Emit a connection status update
 */
export function emitStatus(state: ConnectionState, message?: string): void {
  emit({
    type: 'status',
    state,
    message,
  });
}

/**
 * Emit an error message
 */
export function emitError(message: string): void {
  emit({
    type: 'error',
    message,
  });
}

/**
 * Log a debug message to stderr (won't interfere with stdout JSON)
 */
export function debug(message: string, ...args: any[]): void {
  console.error(`[DEBUG] ${message}`, ...args);
}

/**
 * Log an info message to stderr
 */
export function info(message: string, ...args: any[]): void {
  console.error(`[INFO] ${message}`, ...args);
}

/**
 * Log a warning message to stderr
 */
export function warn(message: string, ...args: any[]): void {
  console.error(`[WARN] ${message}`, ...args);
}

/**
 * Log an error message to stderr
 */
export function error(message: string, ...args: any[]): void {
  console.error(`[ERROR] ${message}`, ...args);
}
