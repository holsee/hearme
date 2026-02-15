//! Audio playback via cpal.
//!
//! Takes decoded PCM f32 samples and plays them through the default output device.
//! Uses a lock-free ring buffer (rtrb) to bridge the async world to the real-time
//! audio callback.

use crate::capture::{CHANNELS, SAMPLE_RATE};
use anyhow::Result;
use cpal::Sample;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Handle to an active playback stream. Drop to stop.
pub struct PlaybackStream {
    _stream: cpal::Stream,
    /// Producer side of the ring buffer. Taken by the decode task.
    producer: Option<rtrb::Producer<f32>>,
}

impl PlaybackStream {
    /// Start playback on the default output device.
    pub fn start() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| anyhow::anyhow!("No output audio device found"))?;

        let config = cpal::StreamConfig {
            channels: CHANNELS,
            sample_rate: SAMPLE_RATE,
            buffer_size: cpal::BufferSize::Default,
        };

        // Ring buffer: ~200ms of audio at 48kHz stereo
        let buffer_size = SAMPLE_RATE as usize * CHANNELS as usize / 5;
        let (producer, mut consumer) = rtrb::RingBuffer::new(buffer_size);

        let stream = device.build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                for sample in data.iter_mut() {
                    *sample = consumer.pop().unwrap_or(Sample::EQUILIBRIUM);
                }
            },
            |err| {
                tracing::error!("Playback error: {err}");
            },
            None,
        )?;

        stream.play()?;

        Ok(Self {
            _stream: stream,
            producer: Some(producer),
        })
    }

    /// Take the producer for use in the decode task.
    /// The PlaybackStream must still be held alive to keep the cpal stream running.
    pub fn take_producer(&mut self) -> rtrb::Producer<f32> {
        self.producer.take().expect("Producer already taken")
    }
}
