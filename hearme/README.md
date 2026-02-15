# hearme

Share your app audio, peer-to-peer.

A minimal Tauri v2 desktop app that captures audio from a specific application and streams it to listeners over iroh (QUIC-based P2P networking with automatic NAT traversal).

## Architecture

```
Sharer                                 Listener(s)
┌──────────────────────────┐          ┌──────────────────────────┐
│ App Audio ──→ capture    │          │ iroh QUIC ──→ Opus       │
│            ──→ Opus enc  │  iroh    │            ──→ decode    │
│            ──→ iroh QUIC │ ◄──────► │            ──→ cpal play │
└──────────────────────────┘  P2P     └──────────────────────────┘
```

- **Capture**: Platform-specific per-app audio capture
  - Linux: PipeWire (`pipewire` crate)
  - macOS: ScreenCaptureKit (`screencapturekit` crate, macOS 13+)
  - Windows: WASAPI process loopback (`wasapi` crate, Windows 10 20348+)
- **Codec**: Opus at 48kHz stereo, 64kbps, 20ms frames
- **Transport**: iroh P2P with QUIC streams, length-prefixed Opus packets
- **Playback**: cpal with lock-free ring buffer (rtrb)
- **1-to-many**: Each listener gets their own QUIC stream via broadcast channel

## System Dependencies

### Linux (Ubuntu/Debian)

```bash
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev \
  libjavascriptcoregtk-4.1-dev \
  libsoup-3.0-dev \
  libpipewire-0.3-dev \
  libopus-dev \
  libasound2-dev \
  libclang-dev \
  pkg-config \
  build-essential
```

### macOS

```bash
# Xcode command line tools (provides clang, frameworks)
xcode-select --install
# Opus (optional — the opus crate builds from source if not found)
brew install opus
```

### Windows

- Visual Studio Build Tools with C++ workload
- Windows 10 SDK (20348+)
- Opus is built from source via cmake

## Build & Run

```bash
cd hearme

# Install npm dependencies (Tauri CLI)
npm install

# Development mode (hot-reload frontend, debug Rust backend)
npm run dev

# Production build
npm run build
```

## Usage

1. **Share**: Select an app producing audio, click "Start Sharing", copy the ticket
2. **Listen**: Paste the ticket, click "Start Listening" — audio plays through your speakers

The iroh transport handles NAT traversal automatically. Direct P2P when possible, relay fallback when not.

## Project Structure

```
hearme/
├── package.json              # Tauri CLI + frontend deps
├── src/
│   └── index.html            # Vanilla JS frontend (no build step)
└── src-tauri/
    ├── Cargo.toml
    ├── tauri.conf.json
    ├── capabilities/
    │   └── default.json
    └── src/
        ├── main.rs           # Binary entry point
        ├── lib.rs            # Tauri app setup
        ├── app.rs            # Tauri commands (start/stop share/listen)
        ├── capture/
        │   ├── mod.rs        # Cross-platform trait + constants
        │   ├── linux.rs      # PipeWire per-app capture
        │   ├── macos.rs      # ScreenCaptureKit per-app capture
        │   └── windows.rs    # WASAPI process loopback capture
        ├── codec.rs          # Opus encode/decode
        ├── transport.rs      # iroh P2P (share session + listen session)
        └── playback.rs       # cpal audio output with ring buffer
```
