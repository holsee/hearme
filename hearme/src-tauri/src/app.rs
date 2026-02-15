//! Application state and Tauri command handlers.
//!
//! This is the glue that connects the UI to the audio capture, codec,
//! transport, and playback modules.

use crate::capture::{self, AudioSource};
use crate::codec;
use crate::playback::PlaybackStream;
use crate::transport::{ListenSession, ShareSession, Ticket};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;
use tracing::{error, info};

/// Shared application state managed by Tauri.
pub struct AppState {
    /// Active sharing session (if any).
    share: Mutex<Option<ShareContext>>,
    /// Active listening session (if any).
    listen: Mutex<Option<ListenContext>>,
}

struct ShareContext {
    session: ShareSession,
    _capture_handle: capture::CaptureHandle,
    encode_task: tokio::task::JoinHandle<()>,
}

struct ListenContext {
    session: ListenSession,
    /// Hold the cpal stream alive. Audio plays as long as this exists.
    _playback: PlaybackStream,
    decode_task: tokio::task::JoinHandle<()>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            share: Mutex::new(None),
            listen: Mutex::new(None),
        }
    }
}

/// List audio sources (applications producing audio).
#[tauri::command]
pub async fn list_audio_sources() -> Result<Vec<AudioSource>, String> {
    capture::list_sources().await.map_err(|e| e.to_string())
}

/// Start sharing audio from the selected source.
/// Returns the ticket string for listeners to connect.
#[tauri::command]
pub async fn start_sharing(
    state: State<'_, AppState>,
    source: AudioSource,
    app: AppHandle,
) -> Result<String, String> {
    let mut share_guard = state.share.lock().await;
    if share_guard.is_some() {
        return Err("Already sharing".into());
    }

    // Start the P2P share session
    let (session, ticket) = ShareSession::start().await.map_err(|e| e.to_string())?;
    let ticket_str = ticket.to_string_encoded().map_err(|e| e.to_string())?;

    info!("Share ticket: {ticket_str}");

    // Start capturing audio from the selected app
    let (capture_handle, mut pcm_rx) = capture::start_capture(&source)
        .await
        .map_err(|e| e.to_string())?;

    // Spawn task: read PCM -> encode Opus -> broadcast to listeners
    let opus_tx = session.opus_tx.clone();
    let app_clone = app.clone();
    let encode_task = tokio::spawn(async move {
        let mut encoder = match codec::Encoder::new() {
            Ok(e) => e,
            Err(e) => {
                error!("Failed to create Opus encoder: {e}");
                return;
            }
        };

        while let Some(pcm_frame) = pcm_rx.recv().await {
            match encoder.encode(&pcm_frame) {
                Ok(packet) => {
                    let _ = opus_tx.send(Arc::new(packet));
                }
                Err(e) => {
                    error!("Opus encode error: {e}");
                }
            }
        }

        info!("Capture stream ended");
        let _ = app_clone.emit("share-ended", ());
    });

    *share_guard = Some(ShareContext {
        session,
        _capture_handle: capture_handle,
        encode_task,
    });

    Ok(ticket_str)
}

/// Stop sharing.
#[tauri::command]
pub async fn stop_sharing(state: State<'_, AppState>) -> Result<(), String> {
    let mut share_guard = state.share.lock().await;
    if let Some(ctx) = share_guard.take() {
        ctx.encode_task.abort();
        ctx.session.stop().await.map_err(|e| e.to_string())?;
        info!("Stopped sharing");
    }
    Ok(())
}

/// Start listening to a sharer by their ticket.
#[tauri::command]
pub async fn start_listening(
    state: State<'_, AppState>,
    ticket_str: String,
    app: AppHandle,
) -> Result<(), String> {
    let mut listen_guard = state.listen.lock().await;
    if listen_guard.is_some() {
        return Err("Already listening".into());
    }

    let ticket = Ticket::from_string_encoded(&ticket_str).map_err(|e| e.to_string())?;

    // Connect to the sharer
    let (session, mut opus_rx) = ListenSession::connect(&ticket)
        .await
        .map_err(|e| e.to_string())?;

    // Start playback — take the producer out for the decode task
    let mut playback = PlaybackStream::start().map_err(|e| e.to_string())?;
    let mut producer = playback.take_producer();

    // Spawn task: receive Opus packets -> decode -> push to ring buffer
    let app_clone = app.clone();
    let decode_task = tokio::spawn(async move {
        let mut decoder = match codec::Decoder::new() {
            Ok(d) => d,
            Err(e) => {
                error!("Failed to create Opus decoder: {e}");
                return;
            }
        };

        while let Some(packet) = opus_rx.recv().await {
            match decoder.decode(&packet) {
                Ok(pcm) => {
                    for &sample in &pcm {
                        // Non-blocking push; if ring buffer is full, drop samples
                        // (better than blocking the async runtime)
                        let _ = producer.push(sample);
                    }
                }
                Err(e) => {
                    error!("Opus decode error: {e}");
                }
            }
        }

        info!("Listen stream ended");
        let _ = app_clone.emit("listen-ended", ());
    });

    *listen_guard = Some(ListenContext {
        session,
        _playback: playback,
        decode_task,
    });

    Ok(())
}

/// Stop listening.
#[tauri::command]
pub async fn stop_listening(state: State<'_, AppState>) -> Result<(), String> {
    let mut listen_guard = state.listen.lock().await;
    if let Some(ctx) = listen_guard.take() {
        ctx.decode_task.abort();
        ctx.session.stop().await;
        info!("Stopped listening");
    }
    Ok(())
}
