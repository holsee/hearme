//! Windows per-app audio capture via WASAPI process loopback.
//!
//! Requires Windows 10 build 20348+ (Windows 11 / Server 2022).
//! Uses WASAPI's AudioClient application loopback mode to capture
//! audio from a specific process by PID.

use super::{AudioSource, CHANNELS, CaptureHandle, SAMPLE_RATE, SAMPLES_PER_FRAME};
use tokio::sync::mpsc;

/// List applications with active audio sessions via WASAPI session enumeration.
/// Runs on a blocking thread because COM/WASAPI objects are `!Send`.
pub async fn list_sources() -> anyhow::Result<Vec<AudioSource>> {
    tokio::task::spawn_blocking(list_sources_sync).await?
}

fn list_sources_sync() -> anyhow::Result<Vec<AudioSource>> {
    use std::collections::HashMap;
    use sysinfo::System;
    use wasapi::*;

    // Initialize COM for this thread
    initialize_mta()
        .ok()
        .map_err(|e| anyhow::anyhow!("COM init failed: {e}"))?;

    // Collect PIDs with active audio sessions from all render devices
    let mut audio_pids: Vec<u32> = Vec::new();
    let enumerator =
        DeviceEnumerator::new().map_err(|e| anyhow::anyhow!("DeviceEnumerator failed: {e}"))?;
    let collection = enumerator
        .get_device_collection(&Direction::Render)
        .map_err(|e| anyhow::anyhow!("get_device_collection failed: {e}"))?;

    for device_result in &collection {
        let device = match device_result {
            Ok(d) => d,
            Err(_) => continue,
        };
        let session_manager = match device.get_iaudiosessionmanager() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let session_enum = match session_manager.get_audiosessionenumerator() {
            Ok(e) => e,
            Err(_) => continue,
        };
        let count = match session_enum.get_count() {
            Ok(c) => c,
            Err(_) => continue,
        };

        for i in 0..count {
            if let Ok(session) = session_enum.get_session(i) {
                let pid = session.get_process_id().unwrap_or(0);
                let state = session.get_state().unwrap_or(SessionState::Expired);
                // PID 0 = system audio engine, skip it
                if pid != 0 && state == SessionState::Active {
                    audio_pids.push(pid);
                }
            }
        }
    }

    if audio_pids.is_empty() {
        return Ok(Vec::new());
    }

    // Resolve PIDs to process names via sysinfo
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let pid_to_name: HashMap<u32, String> = sys
        .processes()
        .iter()
        .map(|(pid, process)| (pid.as_u32(), process.name().to_string_lossy().to_string()))
        .collect();

    let mut sources: Vec<AudioSource> = audio_pids
        .into_iter()
        .filter_map(|pid| {
            let name = pid_to_name.get(&pid)?.clone();
            if name.is_empty() {
                return None;
            }
            Some(AudioSource {
                id: pid.to_string(),
                name,
            })
        })
        .collect();

    sources.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    sources.dedup_by(|a, b| a.name == b.name);
    Ok(sources)
}

pub async fn start_capture(
    source: &AudioSource,
) -> anyhow::Result<(CaptureHandle, mpsc::Receiver<Vec<f32>>)> {
    let pid: u32 = source.id.parse()?;
    let (tx, rx) = mpsc::channel::<Vec<f32>>(64);
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();

    std::thread::spawn(move || {
        if let Err(e) = capture_loop(pid, tx, stop_rx) {
            tracing::error!("WASAPI capture error: {e}");
        }
    });

    Ok((CaptureHandle::new(stop_tx), rx))
}

fn capture_loop(
    pid: u32,
    tx: mpsc::Sender<Vec<f32>>,
    mut stop_rx: tokio::sync::oneshot::Receiver<()>,
) -> anyhow::Result<()> {
    use wasapi::*;

    // Initialize COM for this thread
    initialize_mta()
        .ok()
        .map_err(|e| anyhow::anyhow!("COM init failed: {e}"))?;

    // Create a loopback capture client targeting this process
    let mut audio_client = AudioClient::new_application_loopback_client(pid, true)
        .map_err(|e| anyhow::anyhow!("Failed to create loopback client for PID {pid}: {e}"))?;

    // Request 48kHz stereo f32
    let desired_format = WaveFormat::new(
        32, // bits per sample
        32, // valid bits
        &SampleType::Float,
        SAMPLE_RATE as usize,
        CHANNELS as usize,
        None,
    );

    // Use event-driven shared mode with autoconvert
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: 200_000, // 20ms
    };
    audio_client
        .initialize_client(&desired_format, &Direction::Capture, &mode)
        .map_err(|e| anyhow::anyhow!("Init capture failed: {e}"))?;

    let capture_client = audio_client
        .get_audiocaptureclient()
        .map_err(|e| anyhow::anyhow!("Get capture client failed: {e}"))?;

    let event_handle = audio_client
        .set_get_eventhandle()
        .map_err(|e| anyhow::anyhow!("Event handle failed: {e}"))?;

    audio_client
        .start_stream()
        .map_err(|e| anyhow::anyhow!("Start stream failed: {e}"))?;

    let mut accumulator: Vec<f32> = Vec::with_capacity(SAMPLES_PER_FRAME * 2);
    // bytes per frame: channels * 4 bytes (f32)
    let frame_bytes = CHANNELS as usize * 4;
    // Buffer for ~100ms of audio
    let mut read_buf = vec![0u8; SAMPLE_RATE as usize * frame_bytes / 10];

    loop {
        // Check stop signal
        if stop_rx.try_recv().is_ok() {
            break;
        }

        // Wait for audio data (100ms timeout)
        if event_handle.wait_for_event(100).is_err() {
            continue;
        }

        // Read available frames (interleaved f32 bytes)
        match capture_client.read_from_device(&mut read_buf) {
            Ok((frames_read, _info)) => {
                if frames_read == 0 {
                    continue;
                }
                let bytes_read = frames_read as usize * frame_bytes;
                let samples: &[f32] = bytemuck_cast_slice(&read_buf[..bytes_read]);

                accumulator.extend_from_slice(samples);

                while accumulator.len() >= SAMPLES_PER_FRAME {
                    let frame: Vec<f32> = accumulator.drain(..SAMPLES_PER_FRAME).collect();
                    if tx.blocking_send(frame).is_err() {
                        return Ok(());
                    }
                }
            }
            Err(e) => {
                tracing::warn!("WASAPI read error: {e}");
            }
        }
    }

    audio_client.stop_stream().ok();
    Ok(())
}

fn bytemuck_cast_slice(bytes: &[u8]) -> &[f32] {
    let len = bytes.len() / 4;
    unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const f32, len) }
}
