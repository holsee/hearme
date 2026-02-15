//! Opus encoding and decoding.
//!
//! Wraps the `opus` crate for 48kHz stereo at 64kbps.
//! Frame size: 20ms = 960 samples/channel = 1920 interleaved f32s.

use crate::capture::{CHANNELS, SAMPLES_PER_FRAME};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::FRAME_SIZE;

    #[test]
    fn encode_decode_round_trip_silence() {
        let mut encoder = Encoder::new().expect("encoder creation");
        let mut decoder = Decoder::new().expect("decoder creation");

        // 20ms of silence
        let input = vec![0.0f32; SAMPLES_PER_FRAME];
        let packet = encoder.encode(&input).expect("encode");

        // Opus packets should be non-empty and much smaller than raw PCM
        assert!(!packet.is_empty());
        assert!(packet.len() < SAMPLES_PER_FRAME * 4); // smaller than raw f32 data

        let output = decoder.decode(&packet).expect("decode");
        assert_eq!(output.len(), SAMPLES_PER_FRAME);

        // Silence in should produce near-silence out
        for &sample in &output {
            assert!(sample.abs() < 0.01, "Expected near-silence, got {sample}");
        }
    }

    #[test]
    fn encode_decode_round_trip_sine() {
        let mut encoder = Encoder::new().expect("encoder creation");
        let mut decoder = Decoder::new().expect("decoder creation");

        // Generate a 440Hz sine wave, 20ms, stereo interleaved
        let mut input = vec![0.0f32; SAMPLES_PER_FRAME];
        for i in 0..FRAME_SIZE {
            let t = i as f32 / 48_000.0;
            let sample = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.5;
            input[i * CHANNELS as usize] = sample; // left
            input[i * CHANNELS as usize + 1] = sample; // right
        }

        let packet = encoder.encode(&input).expect("encode");
        assert!(!packet.is_empty());

        let output = decoder.decode(&packet).expect("decode");
        assert_eq!(output.len(), SAMPLES_PER_FRAME);

        // Check that the output has non-trivial energy (not all zeros)
        let energy: f32 = output.iter().map(|s| s * s).sum();
        assert!(
            energy > 1.0,
            "Decoded audio should have significant energy, got {energy}"
        );
    }

    #[test]
    #[should_panic(expected = "Expected")]
    fn encode_rejects_wrong_frame_size() {
        let mut encoder = Encoder::new().expect("encoder creation");
        let wrong_size = vec![0.0f32; SAMPLES_PER_FRAME + 1];
        let _ = encoder.encode(&wrong_size);
    }

    #[test]
    fn multiple_frames_encode_decode() {
        let mut encoder = Encoder::new().expect("encoder creation");
        let mut decoder = Decoder::new().expect("decoder creation");

        // Encode and decode 10 consecutive frames
        for frame_idx in 0..10 {
            let mut input = vec![0.0f32; SAMPLES_PER_FRAME];
            for i in 0..FRAME_SIZE {
                let t = (frame_idx * FRAME_SIZE + i) as f32 / 48_000.0;
                let sample = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.5;
                input[i * CHANNELS as usize] = sample;
                input[i * CHANNELS as usize + 1] = sample;
            }

            let packet = encoder.encode(&input).expect("encode");
            let output = decoder.decode(&packet).expect("decode");
            assert_eq!(output.len(), SAMPLES_PER_FRAME);
        }
    }
}
