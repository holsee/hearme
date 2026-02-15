//! Per-application audio capture abstraction.
//!
//! Each platform has its own mechanism for capturing audio from a specific app:
//! - Linux: PipeWire (attach to an app's audio output node)
//! - macOS: ScreenCaptureKit (per-app audio, macOS 13+)
//! - Windows: WASAPI process loopback (per-PID capture)

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

/// An audio source that can be captured (an application producing audio).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioSource {
    /// Platform-specific identifier (PipeWire node ID, PID, SCK app ID).
    pub id: String,
    /// Human-readable name (e.g. "Firefox", "Spotify").
    pub name: String,
}

/// Audio format we normalize everything to before Opus encoding.
pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: u16 = 2;
/// 20ms frame at 48kHz stereo = 960 samples per channel.
pub const FRAME_SIZE: usize = 960;
/// Interleaved samples per frame: 960 * 2 channels = 1920 f32s.
pub const SAMPLES_PER_FRAME: usize = FRAME_SIZE * CHANNELS as usize;

/// List applications currently producing audio.
pub async fn list_sources() -> anyhow::Result<Vec<AudioSource>> {
    #[cfg(target_os = "linux")]
    return linux::list_sources().await;

    #[cfg(target_os = "macos")]
    return macos::list_sources().await;

    #[cfg(target_os = "windows")]
    return windows::list_sources().await;

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    anyhow::bail!("Unsupported platform for audio capture")
}

/// Start capturing audio from the given source. Returns a receiver of PCM f32
/// frames (each frame is `SAMPLES_PER_FRAME` interleaved f32 samples = 20ms).
/// The returned `CaptureHandle` must be kept alive; dropping it stops capture.
pub async fn start_capture(
    source: &AudioSource,
) -> anyhow::Result<(CaptureHandle, mpsc::Receiver<Vec<f32>>)> {
    #[cfg(target_os = "linux")]
    return linux::start_capture(source).await;

    #[cfg(target_os = "macos")]
    return macos::start_capture(source).await;

    #[cfg(target_os = "windows")]
    return windows::start_capture(source).await;

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    anyhow::bail!("Unsupported platform for audio capture")
}

/// Handle to an active capture session. Drop to stop capture.
pub struct CaptureHandle {
    _stop: tokio::sync::oneshot::Sender<()>,
}

impl CaptureHandle {
    pub fn new(stop: tokio::sync::oneshot::Sender<()>) -> Self {
        Self { _stop: stop }
    }
}
