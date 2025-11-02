# Roon Sidecar - Now Playing

Node.js sidecar process that connects to Roon Core and provides now playing information to the main Tauri application.

## Architecture

This sidecar uses the official Roon API to:
- Auto-discover Roon Core on the local network
- Subscribe to zone updates and track playback state
- Fetch album artwork and convert to base64
- Emit JSON messages to stdout for IPC with the Rust application

## Development

### Install dependencies

```bash
npm install
```

### Build TypeScript

```bash
npm run build
```

### Run standalone (for testing)

```bash
npm start
```

This will:
1. Start discovery of Roon Core on your network
2. Wait for authorization (you'll need to enable the extension in Roon Settings → Extensions)
3. Once authorized, subscribe to zones and emit JSON updates to stdout

### Watch mode (during development)

```bash
npm run dev
```

This runs TypeScript in watch mode, automatically recompiling on changes.

## Testing with Roon

To test the sidecar standalone:

1. Make sure Roon Core is running on your network
2. Run `npm start` in this directory
3. You should see log messages on stderr like:
   ```
   [INFO] Starting Roon client...
   [INFO] Roon discovery started
   ```
4. In Roon, go to **Settings → Extensions**
5. You should see **"Now Playing Menu Bar"** listed
6. Click **Enable** to authorize the extension
7. Play some music in Roon
8. Watch stdout for JSON messages like:
   ```json
   {"type":"status","state":"connected","message":"Connected to Roon Core"}
   {"type":"now_playing","title":"Song Title","artist":"Artist Name","album":"Album Name","state":"playing","artwork":"data:image/jpeg;base64,..."}
   ```

## Output Format

The sidecar emits line-delimited JSON to stdout:

### Status messages
```json
{"type":"status","state":"discovering|connected|disconnected|not_authorized","message":"..."}
```

### Now playing updates
```json
{
  "type": "now_playing",
  "title": "Song Title",
  "artist": "Artist Name",
  "album": "Album Name",
  "state": "playing|paused|stopped",
  "artwork": "data:image/jpeg;base64,..."
}
```

### Error messages
```json
{"type":"error","message":"..."}
```

## Building the Bundled Executable

To create a standalone executable for distribution:

```bash
npm run bundle
```

This uses `pkg` to bundle Node.js and the sidecar into a single executable that will be placed at:
```
../src-tauri/binaries/roon-sidecar-aarch64-apple-darwin
```

The Tauri app will automatically include this binary in the bundle.

## Directory Structure

```
sidecar/
├── src/
│   ├── index.ts              # Entry point
│   ├── output.ts             # JSON output utilities
│   ├── types/                # TypeScript type definitions
│   │   ├── node-roon-api.d.ts
│   │   ├── node-roon-api-transport.d.ts
│   │   └── node-roon-api-image.d.ts
│   └── roon/
│       ├── client.ts         # Main Roon API client
│       ├── transport.ts      # Zone subscriptions
│       └── image.ts          # Artwork fetching
├── build/                    # Compiled JavaScript (gitignored)
├── package.json
├── tsconfig.json
└── README.md
```

## Troubleshooting

### "Could not find Roon Core"
- Ensure Roon Core is running on your local network
- Check your firewall settings
- Make sure multicast/UDP discovery is not blocked

### "Not authorized"
- Go to Roon Settings → Extensions
- Find "Now Playing Menu Bar" and enable it

### No output appearing
- The sidecar only emits JSON when there are updates
- Try playing music to trigger now playing updates
- Check stderr for debug messages

## Dependencies

- **node-roon-api**: Official Roon API client library
- **node-roon-api-transport**: Zone and playback control
- **node-roon-api-image**: Album artwork fetching
