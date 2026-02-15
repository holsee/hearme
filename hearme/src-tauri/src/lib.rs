//! hearme — share your app audio P2P.
//!
//! Architecture:
//!
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │                  Sharer                              │
//! │  [App Audio] → capture → Opus encode → iroh QUIC →  │
//! └───────────────────────────────────┬─────────────────┘
//!                                     │ P2P (hole-punched)
//! ┌───────────────────────────────────▼─────────────────┐
//! │                  Listener                            │
//! │  → iroh QUIC recv → Opus decode → cpal playback     │
//! └─────────────────────────────────────────────────────┘
//! ```

pub mod app;
pub mod capture;
pub mod codec;
pub mod playback;
pub mod transport;

use app::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "hearme=info,iroh=warn".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            app::list_audio_sources,
            app::start_sharing,
            app::stop_sharing,
            app::start_listening,
            app::stop_listening,
        ])
        .run(tauri::generate_context!())
        .expect("error while running hearme");
}
