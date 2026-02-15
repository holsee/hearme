//! Opus encoding and decoding.
//!
//! Wraps the `opus` crate for 48kHz stereo at 64kbps.
//! Frame size: 20ms = 960 samples/channel = 1920 interleaved f32s.

use crate::capture::{CHANNELS, FRAME_SIZE, SAMPLES_PER_FRAME};

/// Maximum Opus packet size (20ms stereo at high bitrate won't exceed this).
const MAX_PACKET_SIZE: usize = 4000;

pub struct Encoder {
    inner: opus::Encoder,
}

impl Encoder {
    pub fn new() -> anyhow::Result<Self> {
        let channels = if CHANNELS == 2 {
            opus::Channels::Stereo
        } else {
            opus::Channels::Mono
        };
        let mut enc = opus::Encoder::new(48_000, channels, opus::Application::Audio)?;
        enc.set_bitrate(opus::Bitrate::Bits(64_000))?;
        Ok(Self { inner: enc })
    }

    /// Encode a 20ms PCM f32 frame into an Opus packet.
    /// Input must be exactly `SAMPLES_PER_FRAME` interleaved f32 samples.
    /// Returns the encoded bytes.
    pub fn encode(&mut self, pcm: &[f32]) -> anyhow::Result<Vec<u8>> {
        assert_eq!(
            pcm.len(),
            SAMPLES_PER_FRAME,
            "Expected {SAMPLES_PER_FRAME} samples, got {}",
            pcm.len()
        );
        let mut output = vec![0u8; MAX_PACKET_SIZE];
        let len = self.inner.encode_float(pcm, &mut output)?;
        output.truncate(len);
        Ok(output)
    }
}

pub struct Decoder {
    inner: opus::Decoder,
}

impl Decoder {
    pub fn new() -> anyhow::Result<Self> {
        let channels = if CHANNELS == 2 {
            opus::Channels::Stereo
        } else {
            opus::Channels::Mono
        };
        let dec = opus::Decoder::new(48_000, channels)?;
        Ok(Self { inner: dec })
    }

    /// Decode an Opus packet into PCM f32 samples.
    /// Returns `SAMPLES_PER_FRAME` interleaved f32 samples.
    pub fn decode(&mut self, packet: &[u8]) -> anyhow::Result<Vec<f32>> {
        let mut output = vec![0f32; SAMPLES_PER_FRAME];
        let decoded = self.inner.decode_float(packet, &mut output, false)?;
        // decoded is samples per channel
        let total = decoded * CHANNELS as usize;
        output.truncate(total);
        Ok(output)
    }
}
