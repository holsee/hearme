# hearme

[![hearme build](https://github.com/holsee/hearme/actions/workflows/hearme-build.yml/badge.svg)](https://github.com/holsee/hearme/actions/workflows/hearme-build.yml)

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
  - Linux: PipeWire (`pipewire` crate with `v0_3_44` feature for `TARGET_OBJECT`)
  - macOS: ScreenCaptureKit (`screencapturekit` crate, macOS 13+)
  - Windows: WASAPI process loopback (`wasapi` crate, Windows 10 20348+)
- **Codec**: Opus at 48kHz stereo, 64kbps, 20ms frames
- **Transport**: iroh 0.96 P2P with QUIC bi-streams, length-prefixed Opus packets (`u16 LE` + bytes)
- **Playback**: cpal 0.17 audio output with lock-free ring buffer (rtrb)
- **Ticket**: `EndpointAddr` serialized to JSON, base64url-encoded for copy-paste
- **1-to-many**: Each listener gets their own QUIC stream via broadcast channel

## Status

The app **compiles and CI passes** on Linux and Windows. macOS CI is ready to enable.

Runtime testing has not been done yet — platform-specific capture code (PipeWire enumeration, ScreenCaptureKit permissions, WASAPI session handling) may need adjustments when first run on real hardware.

## System Dependencies

### Linux (Ubuntu/Debian)

```bash
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev \
  libjavascriptcoregtk-4.1-dev \
  libsoup-3.0-dev \
  libpipewire-0.3-dev \
  libspa-0.2-dev \
  libopus-dev \
  libasound2-dev \
  libclang-dev \
  libgtk-3-dev \
  librsvg2-dev \
  patchelf \
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

## CI

GitHub Actions builds for Linux and Windows run on every push to `main` that touches `hearme/` or the workflow file. macOS is commented out in the workflow, ready to enable when needed.

Build artifacts (`.deb`, `.rpm`, `.AppImage`, `.msi`, `.exe`) are uploaded and downloadable from the [Actions tab](https://github.com/holsee/hearme/actions/workflows/hearme-build.yml).

## Project Structure

```
hearme/
├── .gitignore
├── package.json              # @tauri-apps/cli + @tauri-apps/api
├── README.md
├── src/
│   └── index.html            # Vanilla JS frontend (no build step)
└── src-tauri/
    ├── build.rs              # tauri_build::build()
    ├── Cargo.toml            # All deps with platform-specific sections
    ├── Cargo.lock
    ├── tauri.conf.json       # Tauri v2 config
    ├── capabilities/
    │   └── default.json      # Tauri permissions
    ├── icons/                # 8-bit RGBA PNGs + ICO
    │   ├── icon.png          # 512x512
    │   ├── icon.ico          # 256x256
    │   ├── 32x32.png
    │   ├── 128x128.png
    │   └── 128x128@2x.png
    └── src/
        ├── main.rs           # Binary entry point
        ├── lib.rs            # Tauri app setup, module declarations
        ├── app.rs            # Tauri commands (list/start/stop share/listen)
        ├── capture/
        │   ├── mod.rs        # AudioSource trait + constants (48kHz/stereo/20ms)
        │   ├── linux.rs      # PipeWire per-app capture
        │   ├── macos.rs      # ScreenCaptureKit per-app capture
        │   └── windows.rs    # WASAPI process loopback capture
        ├── codec.rs          # Opus encode/decode (64kbps)
        ├── transport.rs      # iroh P2P (ShareSession + ListenSession + Ticket)
        └── playback.rs       # cpal audio output with rtrb ring buffer
```
