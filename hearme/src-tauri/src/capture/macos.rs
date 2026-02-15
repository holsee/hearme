//! macOS per-app audio capture via ScreenCaptureKit.
//!
//! Requires macOS 13+ (Ventura) for audio capture.
//! ScreenCaptureKit can capture audio from a specific application without
//! any virtual audio device.

use super::{AudioSource, CHANNELS, CaptureHandle, SAMPLE_RATE, SAMPLES_PER_FRAME};
use tokio::sync::mpsc;

pub async fn list_sources() -> anyhow::Result<Vec<AudioSource>> {
    use screencapturekit::shareable_content::SCShareableContent;

    let content = SCShareableContent::get()
        .map_err(|e| anyhow::anyhow!("Failed to get shareable content: {e:?}"))?;

    let sources = content
        .applications
        .iter()
        .filter(|app| !app.bundle_identifier.is_empty())
        .map(|app| AudioSource {
            id: app.bundle_identifier.clone(),
            name: app
                .application_name
                .clone()
                .unwrap_or_else(|| app.bundle_identifier.clone()),
        })
        .collect();

    Ok(sources)
}

pub async fn start_capture(
    source: &AudioSource,
) -> anyhow::Result<(CaptureHandle, mpsc::Receiver<Vec<f32>>)> {
    use screencapturekit::{
        content_filter::{InitParams, SCContentFilter},
        output::SCStreamOutputType,
        shareable_content::SCShareableContent,
        stream::{SCStream, SCStreamConfiguration},
    };

    let content = SCShareableContent::get()
        .map_err(|e| anyhow::anyhow!("Failed to get shareable content: {e:?}"))?;

    // Find the target application
    let app = content
        .applications
        .iter()
        .find(|a| a.bundle_identifier == source.id)
        .ok_or_else(|| anyhow::anyhow!("Application '{}' not found", source.name))?
        .clone();

    // Create a content filter for this app (audio only, no video)
    let filter = SCContentFilter::new(InitParams::DesktopIndependentWindow(
        // We need at least one window from the app for the filter
        content
            .windows
            .iter()
            .find(|w| {
                w.owning_application
                    .as_ref()
                    .map_or(false, |a| a.bundle_identifier == source.id)
            })
            .ok_or_else(|| anyhow::anyhow!("No windows found for '{}'", source.name))?
            .clone(),
    ));

    // Configure for audio-only capture
    let config = SCStreamConfiguration {
        captures_audio: true,
        sample_rate: SAMPLE_RATE,
        channel_count: CHANNELS as u32,
        width: 1, // Minimal video (can't fully disable)
        height: 1,
        ..Default::default()
    };

    let (tx, rx) = mpsc::channel::<Vec<f32>>(64);
    let (stop_tx, _stop_rx) = tokio::sync::oneshot::channel::<()>();

    // Create and start the stream
    let mut stream = SCStream::new(&filter, &config);

    // Add output handler for audio samples
    let accumulator = std::sync::Arc::new(std::sync::Mutex::new(Vec::with_capacity(
        SAMPLES_PER_FRAME * 2,
    )));
    let acc_clone = accumulator.clone();
    let tx_clone = tx.clone();

    stream.add_output_handler(SCStreamOutputType::Audio, move |sample_buffer| {
        // Extract PCM f32 data from CMSampleBuffer
        if let Some(audio_buffer) = sample_buffer.audio_buffer_list() {
            for buffer in audio_buffer.buffers() {
                let samples: &[f32] = unsafe {
                    std::slice::from_raw_parts(
                        buffer.data.as_ptr() as *const f32,
                        buffer.data.len() / 4,
                    )
                };

                let mut acc = acc_clone.lock().unwrap();
                acc.extend_from_slice(samples);

                while acc.len() >= SAMPLES_PER_FRAME {
                    let frame: Vec<f32> = acc.drain(..SAMPLES_PER_FRAME).collect();
                    let _ = tx_clone.try_send(frame);
                }
            }
        }
    });

    stream
        .start_capture()
        .map_err(|e| anyhow::anyhow!("Failed to start capture: {e:?}"))?;

    Ok((CaptureHandle::new(stop_tx), rx))
}
