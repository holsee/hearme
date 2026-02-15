# BISC2 -- Technical Product Requirements Document

## P2P Coworking Application Built on Iroh

**Status:** Draft
**Date:** 2026-02-15

---

## 1. Vision

A native desktop application for serious coworking -- combining the social
structure of Discord (servers, channels, presence) with the real-time media
capabilities of a video conferencing tool (camera, screen share, voice, app
audio). Fully peer-to-peer, no central servers required beyond relay-assisted
NAT traversal.

The application is built entirely on the **iroh** networking stack by n0, using
proven primitives from [iroh-examples](https://github.com/n0-computer/iroh-examples)
and the experimental [iroh-live](https://github.com/n0-computer/iroh-live) media
layer.

---

## 2. Core Technology Stack

### 2.1 Iroh (Networking Foundation)

- **Repository:** https://github.com/n0-computer/iroh (v0.96, 7.8k stars)
- **Transport:** QUIC over UDP via the `quinn` crate, with TLS 1.3 mutual authentication
- **Identity:** Ed25519 keypairs -- public keys serve as node identifiers (`EndpointId`)
- **Connectivity:** Automatic NAT hole-punching with relay server fallback; connections
  upgrade from relayed to direct P2P transparently
- **Protocol multiplexing:** ALPN-based; multiple protocols share one endpoint via a `Router`
- **Discovery:** DNS discovery (`DnsDiscovery::n0_dns()`) and pkarr publishing

### 2.2 Iroh-Live (Media Layer)

- **Repository:** https://github.com/n0-computer/iroh-live (experimental, 46 stars)
- **Protocol:** Media over QUIC (MoQ) via [moq-lite](https://github.com/kixelated/moq)
- **Bridge:** [web-transport-iroh](https://github.com/n0-computer/web-transport-iroh)
  adapts iroh QUIC connections to the WebTransport interface MoQ expects
- **Capabilities:**
  - Camera capture via [nokhwa](https://github.com/l1npengtul/nokhwa/)
  - Screen capture via [xcap](https://github.com/nashaofu/xcap/)
  - Audio capture and playout via [firewheel](https://github.com/BillyDM/Firewheel/)
  - H.264 video encoding/decoding via ffmpeg (hardware-accelerated where supported)
  - Opus audio encoding/decoding via ffmpeg
  - Multiple renditions with on-demand switching
  - Room-based multi-party sessions with ticket invites

### 2.3 Supporting Iroh Protocols

| Protocol | Crate | Purpose |
|----------|-------|---------|
| iroh-gossip | `iroh-gossip` | Pub/sub messaging for text chat, presence, signaling |
| iroh-blobs | `iroh-blobs` | Content-addressed file transfer (share files, images, recordings) |
| iroh-docs | `iroh-docs` | Multi-writer document sync (channel lists, user profiles, shared state) |
| Automerge | `automerge` | CRDT-based conflict-free collaborative state (optional) |

---

## 3. Proven Patterns (Reference Examples)

The following examples from [iroh-examples](https://github.com/n0-computer/iroh-examples)
have been validated and demonstrate the building blocks we will use:

### 3.1 Text Chat & Presence

**Reference:** `iroh-examples/browser-chat/`

- `iroh-gossip` with topic-based pub/sub (`TopicId` per channel)
- Cryptographically signed messages (`SecretKey::sign`, `PublicKey::verify`)
- Presence broadcasts every 5 seconds with nickname
- Serialized tickets (`ChatTicket`) for channel join/create
- Wire format: `postcard`-serialized `SignedMessage` with `from`, `data`, `signature`

**Key source:** `browser-chat/shared/src/lib.rs` -- `ChatNode` struct

### 3.2 File Sharing

**Reference:** `iroh-examples/browser-blobs/`

- `iroh-blobs` with `BlobTicket` (endpoint address + BLAKE3 hash + format)
- In-memory (`MemStore`) and filesystem (`FsStore`) backends
- Content-addressed: identical files deduplicate automatically

### 3.3 Persistent Shared State

**Reference:** `iroh-examples/tauri-todos/`

- `iroh-docs` for multi-writer key-value document sync
- `DocTicket` with read/write modes for sharing
- Live event subscription (`LiveEvent::InsertRemote`, `InsertLocal`, `ContentReady`)
- Combines blobs, gossip, and docs protocols via the `Router`

**Reference:** `iroh-examples/iroh-automerge/`

- Automerge CRDT sync over iroh bidirectional streams
- Custom ALPN: `iroh/automerge/2`
- Length-prefixed message exchange until convergence

### 3.4 Custom Protocol Patterns

**Reference:** `iroh-examples/custom-router/`

- Dynamic protocol registration/removal at runtime
- `ProtocolHandler` trait implementation pattern

**Reference:** `iroh-examples/framed-messages/`

- `tokio-util` `LengthDelimitedCodec` for message framing
- `postcard` + `serde` binary serialization

### 3.5 Media Streaming

**Reference:** `iroh-live/iroh-live/examples/rooms.rs`

- Multi-party video + audio room with ticket-based invites
- Camera, screen, and audio capture
- H.264 + Opus encoding over MoQ sessions on iroh connections
- egui-based rendering (framework-agnostic underneath)

---

## 4. Application Architecture

### 4.1 High-Level Components

```
+-------------------------------------------------------------------+
|                        BISC2 Desktop App                          |
|-------------------------------------------------------------------|
|  UI Layer (egui or Tauri + web frontend)                          |
|-------------------------------------------------------------------|
|  Application Logic                                                |
|  +------------------+  +------------------+  +------------------+ |
|  |  Workspace Mgmt  |  |  Channel Mgmt   |  |  User Profiles   | |
|  +------------------+  +------------------+  +------------------+ |
|-------------------------------------------------------------------|
|  Protocol Layer                                                   |
|  +----------+  +--------+  +---------+  +-------+  +-----------+ |
|  | iroh-live |  | gossip |  |  blobs  |  | docs  |  | automerge | |
|  | (MoQ)    |  |        |  |         |  |       |  | (optional)| |
|  +----------+  +--------+  +---------+  +-------+  +-----------+ |
|-------------------------------------------------------------------|
|  Iroh Endpoint (single QUIC endpoint, ALPN multiplexing)          |
|  +-------------------------------------------------------------+ |
|  | Router: registers all protocol handlers by ALPN              | |
|  | Identity: Ed25519 keypair (persistent across sessions)       | |
|  | Discovery: DNS + pkarr for node resolution                   | |
|  | Connectivity: direct P2P with relay fallback                 | |
|  +-------------------------------------------------------------+ |
+-------------------------------------------------------------------+
```

### 4.2 Data Model

```
Workspace (= "server" in Discord terms)
├── metadata: name, icon, description (synced via iroh-docs)
├── invite: WorkspaceTicket (contains DocTicket + bootstrap peers)
├── members: list of EndpointIds with roles
├── channels[]:
│   ├── TextChannel
│   │   ├── topic: TopicId (for iroh-gossip)
│   │   ├── messages: gossip-broadcast SignedMessages
│   │   └── history: iroh-docs or automerge document
│   ├── VoiceChannel
│   │   ├── topic: TopicId (for signaling/presence)
│   │   └── media: iroh-live MoQ sessions (per participant)
│   └── MediaChannel (screen share / cam / mixed)
│       ├── topic: TopicId (for signaling/presence)
│       └── streams[]: iroh-live MoQ tracks
│           ├── camera (H.264)
│           ├── screen (H.264)
│           └── audio (Opus)
└── shared_files: iroh-blobs collection
```

### 4.3 Protocol ALPN Registry

| ALPN | Protocol | Purpose |
|------|----------|---------|
| `iroh-gossip/0` | iroh-gossip | Text chat, presence, signaling |
| `iroh-blobs/...` | iroh-blobs | File transfer |
| `iroh-docs/...` | iroh-docs | Document sync |
| `moq-lite/...` | MoQ via iroh-moq | Media streaming |
| `bisc2/workspace/1` | Custom | Workspace metadata exchange |

---

## 5. Feature Specifications

### 5.1 Workspaces

A workspace is a persistent group of users and channels, analogous to a Discord
server. It is represented as an `iroh-docs` document containing:

- Workspace metadata (name, description, icon blob hash)
- Channel list (name, type, TopicId)
- Member list (EndpointId, display name, role)

**Invite flow:**
1. Creator generates a `WorkspaceTicket` containing the `DocTicket` for the
   workspace document plus bootstrap peer addresses
2. Invitee receives the ticket (out of band: paste, QR code, link)
3. Invitee's node syncs the workspace document via iroh-docs
4. Invitee's EndpointId is added to the member list

**Reference pattern:** `tauri-todos` uses `DocTicket` for exactly this kind of
shared-state invite flow.

### 5.2 Text Channels

Each text channel maps to a `TopicId` in iroh-gossip.

**Real-time messages:** Broadcast via gossip with signed messages (as in
`browser-chat`). Messages contain: sender EndpointId, nickname, text, timestamp,
signature.

**Message persistence:** Messages are additionally written to an iroh-docs
document or Automerge document keyed by `(channel_id, timestamp, sender)`. This
allows late-joining peers to catch up on history.

**Presence:** Periodic gossip broadcasts (every 5s) per channel topic, including
user nickname, status, and active channel.

### 5.3 Voice Channels

A voice channel uses iroh-live's MoQ transport for audio.

**Join flow:**
1. User joins the channel's gossip topic (for signaling/presence)
2. User announces their audio MoQ track via gossip
3. Other participants subscribe to the announced track
4. Audio is captured via firewheel, encoded as Opus, streamed via MoQ

**Reference:** `iroh-live/examples/rooms.rs` demonstrates this exact flow for
audio + video.

### 5.4 Video / Screen Sharing

Video and screen sharing extend the voice channel model with additional MoQ tracks.

**Per-user streams:**
- Camera: captured via nokhwa, encoded as H.264
- Screen: captured via xcap, encoded as H.264
- App window: xcap supports window-specific capture on supported platforms

Each stream is a separate MoQ track, allowing participants to independently
subscribe/unsubscribe to individual streams. iroh-live supports multiple
renditions with on-demand quality switching.

**Multi-stream scenario (coworking):**
A single user can simultaneously publish:
- 1x camera track (low-res talking head)
- 1x screen share track (high-res workspace)
- 1x audio track (microphone via Opus)

Other participants selectively subscribe based on their interest and bandwidth.

### 5.5 File Sharing

Files shared in text channels are transferred via iroh-blobs.

**Flow:**
1. Sender adds file to local blob store
2. Sender broadcasts a `BlobTicket` (hash + address) via the channel's gossip topic
3. Recipients can download the blob on demand
4. Content-addressed: identical files sent by different users are deduplicated

**Reference:** `browser-blobs` demonstrates this ticket-based blob sharing.

### 5.6 Shared Documents / Collaborative State

For features like shared whiteboards, collaborative notes, or task boards:

- Use Automerge CRDTs synced over iroh connections
- Each collaborative document gets its own `DocTicket`
- Changes merge automatically without conflicts

**Reference:** `iroh-automerge` and `iroh-automerge-repo` demonstrate this.

---

## 6. Technical Considerations

### 6.1 Connection Management

- Single `iroh::Endpoint` per application instance
- `Router` multiplexes all protocols (gossip, blobs, docs, MoQ) over one endpoint
- Connections are lazy: established on first interaction with a peer
- Automatic reconnection after network changes (iroh handles connection healing)
- Direct P2P when possible; relay fallback is transparent

### 6.2 Identity & Security

- Each user has a persistent Ed25519 keypair stored locally
- All connections are mutually authenticated via TLS 1.3
- All gossip messages are signed by the sender's secret key and verified by recipients
- Workspace membership can be enforced by checking EndpointIds against the member list
- No central authority: trust is based on cryptographic identity

### 6.3 Media Performance

- H.264 encoding with hardware acceleration (VA-API on Linux, VideoToolbox on macOS)
- Opus audio at 48kHz (configurable bitrate)
- MoQ provides congestion-aware media delivery over QUIC
- Multiple renditions allow bandwidth adaptation
- iroh's QUIC multipath support (in development, see blog post "iroh on QUIC Multipath")
  could improve resilience

### 6.4 Offline Behavior

- Text message history persists locally via iroh-docs / Automerge
- Late-joining peers sync missed messages from any online peer holding the document
- Shared files remain available as long as at least one peer with the blob is online
- Voice/video are inherently real-time and not persisted (unless recording is added)

### 6.5 Platform Support

iroh supports: Linux, macOS, Windows, Android, iOS (via FFI).
iroh-live dependencies:
- nokhwa (camera): Linux, macOS, Windows
- xcap (screen): Linux, macOS, Windows
- firewheel (audio): Linux, macOS, Windows
- ffmpeg: all major platforms

**Initial target:** Linux and macOS desktop.

---

## 7. UI Framework Options

| Option | Pros | Cons |
|--------|------|------|
| **egui** (used by iroh-live examples) | Pure Rust, immediate mode, already proven with iroh-live, simple | Limited widget ecosystem, less polished look |
| **Tauri** (used by tauri-todos example) | Web tech frontend (React/Vue/Svelte), native shell, proven with iroh | Adds web runtime overhead, media rendering via web layer is complex |
| **iced** | Pure Rust, retained mode, good for complex UIs | Less proven with media rendering |

**Recommendation:** Start with **egui** since iroh-live's examples already render
video frames through it. Migrate to Tauri or iced later if richer UI is needed.

---

## 8. Development Phases

### Phase 1: Foundation
- Single workspace with text channels
- Gossip-based chat with signed messages and presence
- Persistent identity (keypair stored on disk)
- Ticket-based invites
- **Build on:** `browser-chat` patterns (adapted to native)

### Phase 2: Voice & Video
- Voice channels using iroh-live audio (Opus over MoQ)
- Camera video in voice channels (H.264 over MoQ)
- Per-participant mute/unmute controls
- **Build on:** `iroh-live/examples/rooms.rs`

### Phase 3: Screen Sharing & Multi-Stream
- Screen capture publishing (xcap)
- Window-specific capture where supported
- Simultaneous camera + screen + audio per user
- Selective stream subscription per viewer
- **Build on:** `iroh-live` screen capture support

### Phase 4: Persistence & Files
- Message history via iroh-docs
- File sharing via iroh-blobs with in-channel previews
- Workspace state persistence (channel list, members)
- **Build on:** `tauri-todos`, `browser-blobs` patterns

### Phase 5: Collaborative Features
- Shared documents / whiteboards via Automerge
- Workspace roles and permissions
- Multiple workspaces
- **Build on:** `iroh-automerge` patterns

---

## 9. Key Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `iroh` | 0.96+ | Core networking, endpoint, connections |
| `iroh-gossip` | latest | Pub/sub messaging |
| `iroh-blobs` | latest | Content-addressed file transfer |
| `iroh-docs` | latest | Document sync |
| `iroh-live` | experimental | Media capture, encoding, MoQ transport |
| `iroh-moq` | experimental | MoQ session adapters for iroh |
| `moq-lite` | latest | MoQ protocol implementation |
| `web-transport-iroh` | latest | WebTransport bridge for iroh connections |
| `nokhwa` | latest | Camera capture |
| `xcap` | latest | Screen capture |
| `firewheel` | latest | Audio capture/playout |
| `ffmpeg-next` | latest | Video/audio encoding/decoding |
| `egui` | latest | UI framework |
| `postcard` | latest | Binary serialization |
| `serde` | latest | Serialization framework |
| `tokio` | 1.x | Async runtime |

---

## 10. Open Questions & Risks

1. **iroh-live maturity:** Labeled "experimental / work in progress" with known bugs.
   How stable is it for daily coworking use?

2. **Multi-stream per user:** Can iroh-live handle simultaneous camera + screen +
   audio tracks from one participant without excessive CPU/bandwidth?

3. **Per-app audio capture:** xcap handles screen/window capture, but capturing
   audio output from a specific application (e.g., music player) is an OS-level
   challenge not addressed by any iroh crate. Platform-specific solutions
   (PulseAudio/PipeWire on Linux, Core Audio on macOS) would be needed.

4. **Relay bandwidth for media:** If peers can't establish direct connections,
   media streams flow through relay servers. What's the bandwidth/latency cost?
   The discussion at https://github.com/n0-computer/iroh/discussions/3815
   reports relay performance concerns.

5. **Browser support (future):** iroh-live is native-only. If a web client is
   ever desired, the media layer would need to be rethought (WebRTC for browser
   media APIs, or WASM ffmpeg which is heavy).

6. **Scaling:** iroh-gossip and MoQ are designed for small groups. What's the
   practical limit for participants in a single room? Likely fine for coworking
   (2-20 people), but untested at scale.

7. **Recording:** No recording capability exists in iroh-live. Adding session
   recording would require muxing decoded streams to disk (ffmpeg can do this).

---

## 11. References

- iroh documentation: https://docs.iroh.computer/
- iroh-examples repository: https://github.com/n0-computer/iroh-examples
- iroh-live repository: https://github.com/n0-computer/iroh-live
- web-transport-iroh: https://github.com/n0-computer/web-transport-iroh
- moq-dev/moq (MoQ implementation): https://github.com/kixelated/moq
- Iroh & the Web (blog): https://www.iroh.computer/blog/iroh-and-the-web
- Iroh on QUIC Multipath (blog): https://www.iroh.computer/blog/iroh-on-QUIC-multipath
- Iroh 1.0 Roadmap: https://www.iroh.computer/blog/road-to-1-0
