# Now Playing - macOS Menu Bar App

A macOS menu bar application that displays currently playing music from [Roon](https://roonlabs.com/) in your menu bar. Built with [Tauri](https://tauri.app/) (Rust) and Node.js.

![License](https://img.shields.io/badge/license-TBD-blue)
![Platform](https://img.shields.io/badge/platform-macOS-lightgrey)

## Features

- **Real-time Display**: Shows currently playing track information (title, artist, album) directly in your macOS menu bar
- **Album Artwork**: Displays album art thumbnail alongside track information
- **Automatic Discovery**: Automatically discovers and connects to Roon Core on your network
- **Manual Connection**: Supports direct connection to Roon Core via environment variables
- **System Integration**: Adapts text color based on macOS appearance (dark/light mode)
- **Retina Support**: High-resolution rendering for crisp text and images on Retina displays

## How It Works

The application consists of two components:

1. **Tauri App (Rust)**: Manages the macOS menu bar icon and renders the display

   - Image compositor with album art and text rendering
   - System tray integration
   - Helvetica Neue font for native macOS appearance
   - Automatic dark/light mode detection

2. **Node.js Sidecar**: Connects to Roon Core and streams playback data
   - Roon API integration for real-time updates
   - Auto-discovery of Roon Core on local network
   - Album artwork fetching and encoding
   - JSON-based communication with Rust app

## Prerequisites

- macOS 10.15 (Catalina) or later
- [Roon Core](https://roonlabs.com/) running on your network
- Rust 1.90.0+ (for building from source)
- Node.js 20.18.0+ (for building from source)
- Xcode Command Line Tools

## Installation

### From Source

1. Clone the repository:

```bash
git clone https://github.com/yourusername/now-playing.git
cd now-playing
```

2. Install dependencies:

```bash
npm install
cd sidecar && npm install && cd ..
```

3. Build the sidecar:

```bash
cd sidecar && npm run build && cd ..
```

4. Run in development mode:

```bash
npm run dev
```

5. Build for production:

```bash
npm run build
```

The built application will be in `src-tauri/target/release/bundle/`.

## Usage

### First Run

1. Launch the application
2. Look for the menu bar icon in the top-right corner of your screen
3. The app will automatically search for Roon Core on your network
4. When found, you'll need to authorize the extension in Roon:
   - Open Roon
   - Go to Settings → Extensions
   - Find "Now Playing Menu Bar" and click "Enable"

Once authorized, the menu bar will display your currently playing track.

### Manual Connection

If auto-discovery doesn't work, you can manually specify your Roon Core address:

```bash
ROON_HOST=192.168.1.100 npm run dev
```

Or for a built app:

```bash
ROON_HOST=192.168.1.100 ROON_PORT=9100 /path/to/Now\ Playing.app/Contents/MacOS/Now\ Playing
```

### Menu Bar Display

The menu bar shows:

- Album artwork (22x22px thumbnail)
- Track title and artist name
- Automatically truncates long titles with ellipsis
- Updates in real-time as tracks change

## Project Structure

```
now-playing/
├── src-tauri/              # Rust/Tauri application
│   ├── src/
│   │   ├── main.rs         # Application entry point
│   │   ├── compositor.rs   # Image generation & text rendering
│   │   ├── sidecar.rs      # Node.js sidecar process management
│   │   ├── tray.rs         # System tray management
│   │   ├── state.rs        # Application state
│   │   └── types.rs        # Data types
│   ├── assets/
│   │   └── fonts/
│   │       └── HelveticaNeue.ttc  # System font for native appearance
│   └── icons/              # Application icons
├── sidecar/                # Node.js Roon API integration
│   ├── src/
│   │   ├── index.ts        # Sidecar entry point
│   │   ├── output.ts       # JSON output formatting
│   │   └── roon/
│   │       ├── client.ts   # Roon API client
│   │       ├── transport.ts # Transport/playback management
│   │       └── image.ts    # Album artwork handling
│   └── package.json
└── package.json
```

## Technical Details

### Architecture

The application uses a **sidecar architecture** where:

- The Rust app manages the UI and system integration
- The Node.js sidecar handles Roon API communication
- Communication happens via stdout/stdin using JSON messages

### Dependencies

**Rust:**

- `tauri 2.0` - Application framework with system tray support
- `image 0.25` - Image manipulation
- `imageproc 0.25` - Drawing primitives
- `ab_glyph 0.2` - Font rendering
- `tokio 1.0` - Async runtime

**Node.js:**

- `node-roon-api` - Roon Core discovery and connection
- `node-roon-api-transport` - Playback state monitoring
- `node-roon-api-image` - Album artwork fetching

### Performance

- **Memory usage**: ~80 MB idle
- **CPU usage**: <1% during normal operation
- **Icon generation**: <10ms per update
- **Network**: Minimal bandwidth (only metadata and small album art thumbnails)

## Development

### Running Tests

```bash
npm test
```

### Building the Sidecar

```bash
npm run build:sidecar
```

This creates a standalone binary that's bundled with the Tauri app.

### Debugging

Enable verbose logging:

```bash
RUST_LOG=debug npm run dev
```

The sidecar logs to stderr, which is captured and displayed by the Rust app.

## Troubleshooting

### App doesn't appear in menu bar

- Check terminal output for errors
- Ensure Roon Core is running and accessible
- Verify the sidecar built successfully (`sidecar/build/index.js` exists)

### Can't connect to Roon Core

- Ensure Roon Core is on the same network
- Check firewall settings (Roon uses port 9100)
- Try manual connection with `ROON_HOST` environment variable
- Verify the extension is enabled in Roon Settings → Extensions

### No album artwork

- Album artwork requires the Roon API Image service
- Some tracks may not have artwork available
- A purple placeholder square is shown when artwork is unavailable

### Text color issues

- The app automatically detects macOS appearance mode
- If text is hard to read, check System Preferences → General → Appearance

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

TBD

## Acknowledgments

- Built with [Tauri](https://tauri.app/)
- Uses [Roon API](https://github.com/RoonLabs/node-roon-api)
- Font: Helvetica Neue (macOS system font)

## Related Projects

- [Roon Labs](https://roonlabs.com/) - The music player this integrates with
- [Roon API Documentation](https://github.com/RoonLabs/node-roon-api)
