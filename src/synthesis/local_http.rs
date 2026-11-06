//! `LocalHttpSynthesizer`: the real [`Synthesizer`] over the Docker
//! Piper HTTP server (`docker/piper/`, bound to `127.0.0.1:5002`).
//!
//! Same shape as the faster-whisper client: a pure helper turns the
//! HTTP response into data. Piper replies with a WAV
//! (`RIFF`-wrapped 16-bit PCM), so [`parse_wav_body`] walks the
//! header and yields raw `f32` samples at the voice's native rate
//! (22050 Hz mono for the `medium` voices), and [`resample_linear`] is
//! the pure, tested rate adapter the CpalPlayer uses to feed a device
//! running at a different rate.
//!
//! The HTTP call itself is glue per DEVELOPMENT_RULES.md: it is
//! verified by `cargo run -- say-text` and the live `piper` path, not
//! by unit tests.

use anyhow::{Context, Result};
use serde_json::json;

use crate::config;
use crate::synthesis::{AudioChunk, Speech, Synthesizer};

/// HTTP client for the local Piper TTS server.
///
/// Constructed from the centralised constants only (`TTS_BASE_URL`,
/// `TTS_VOICE`, `TTS_TIMEOUT_SECS`): no key involved, the server is
/// loopback-only.
pub struct LocalHttpSynthesizer {
    base_url: String,
    voice: String,
    timeout: std::time::Duration,
}

impl LocalHttpSynthesizer {
    /// Build the client from the centralised constants (`TTS_BASE_URL`,
    /// `TTS_VOICE`, `TTS_TIMEOUT_SECS`). No network, no Docker:
    /// construction always succeeds; failures happen on the first
    /// `synthesize` call.
    pub fn from_env() -> Self {
        Self {
            base_url: config::TTS_BASE_URL.to_string(),
            voice: config::TTS_VOICE.to_string(),
            timeout: std::time::Duration::from_secs(config::TTS_TIMEOUT_SECS),
        }
    }
}

impl Synthesizer for LocalHttpSynthesizer {
    fn synthesize(&self, speech: &Speech) -> Result<Option<AudioChunk>> {
        let text = speech.text.trim();
        if text.is_empty() {
            return Ok(None);
        }

        let url = format!("{}/synthesize", self.base_url);
        let body = json!({ "text": text, "voice": self.voice });
        let bytes = reqwest::blocking::Client::builder()
            .timeout(self.timeout)
            .build()
            .context("build the TTS HTTP client")?
            .post(&url)
            .json(&body)
            .send()
            .and_then(|resp| resp.error_for_status())
            .with_context(|| {
                format!(
                    "the TTS server at {} did not answer: is the Docker container up? \
                     docker compose -f docker/piper/docker-compose.yml up -d",
                    self.base_url
                )
            })?
            .bytes()
            .context("read the TTS server response body")?;

        let mut chunk = parse_wav_body(&bytes).context("parse the TTS server WAV response")?;
        // One blocking request per utterance: the reply is the whole
        // utterance, always final (partials are the streaming upgrade
        // in the plan's section 4, not here).
        chunk.is_final = true;
        Ok(Some(chunk))
    }
}

/// Parse Piper's WAV response into an [`AudioChunk`].
///
/// Walks the RIFF chunk list (not fixed offsets) because the chunk
/// order is not guaranteed: reads `WAVE` at the top, then finds the
/// `fmt ` chunk (16-bit PCM expected) and the `data` chunk. The
/// returned chunk carries the rate and channel count the file
/// declares, so rate/carrier adapters downstream can trust it.
/// Invalid magic, a truncated header, a non-PCM format, a depth other
/// than 16 bits, a payload shorter than declared, or a missing
/// chunk is an error, never a panic.
///
/// Pure: bytes in, chunk out. Unit-tested.
pub fn parse_wav_body(body: &[u8]) -> Result<AudioChunk> {
    let riff_ok = body.len() >= 12 && &body[0..4] == b"RIFF" && &body[8..12] == b"WAVE";
    if !riff_ok {
        anyhow::bail!("response is not a RIFF/WAVE file ({} bytes)", body.len());
    }

    // The file body starts right after the 12-byte master header.
    let fmt = fmt_chunk(body)?;
    let samples = pcm_samples(body)?;
    // Samples must be whole channel frames, or the chunk's stated
    // channel count and its sample count disagree and adapters fold
    // misaligned audio instead of failing loudly.
    anyhow::ensure!(
        samples.len() % fmt.channels as usize == 0,
        "WAV data chunk does not line up with the declared channel count"
    );

    Ok(AudioChunk {
        samples,
        sample_rate: fmt.sample_rate,
        channels: fmt.channels,
        is_final: true,
    })
}

struct WavFmt {
    sample_rate: u32,
    channels: u16,
}

fn fmt_chunk(body: &[u8]) -> Result<WavFmt> {
    let mut offset = 12usize;
    while offset + 8 <= body.len() {
        let id = &body[offset..offset + 4];
        let size = u32::from_le_bytes(body[offset + 4..offset + 8].try_into()?) as usize;
        if id == b"fmt " {
            anyhow::ensure!(size >= 16, "WAV fmt chunk is truncated");
            let start = offset + 8;
            // Declared size can lie: a body cut mid-payload would make
            // the field reads below slice past the end and panic, and
            // a size larger than the remaining bytes would leave the
            // data walker decoding bytes that belong to the fmt
            // payload. Both are treated as a bad response, never a
            // crash or half-parsed speech.
            anyhow::ensure!(
                start + size <= body.len(),
                "WAV fmt chunk payload is truncated"
            );
            let audio_format = u16::from_le_bytes(body[start..start + 2].try_into()?);
            anyhow::ensure!(
                audio_format == 1,
                "WAV is not 16-bit integer PCM (format tag {audio_format})"
            );
            let channels = u16::from_le_bytes(body[start + 2..start + 4].try_into()?);
            let sample_rate = u32::from_le_bytes(body[start + 4..start + 8].try_into()?);
            let bits = u16::from_le_bytes(body[start + 14..start + 16].try_into()?);
            anyhow::ensure!(channels > 0, "WAV declares 0 channels");
            anyhow::ensure!(sample_rate > 0, "WAV declares a 0 Hz sample rate");
            // A tiny declared rate makes resample_linear expand the
            // samples by rate/target: a 1 Hz header turns a small
            // reply into gigabytes at playback time. Speech rates live
            // between 8 kHz and 192 kHz; anything else is a lie.
            anyhow::ensure!(
                (8_000..=192_000).contains(&sample_rate),
                "WAV declares a {sample_rate} Hz sample rate; only 8 kHz to 192 kHz is plausible for speech"
            );
            // pcm_samples decodes exactly two-byte signed samples, so a
            // different depth must be refused, not misread.
            anyhow::ensure!(
                bits == 16,
                "WAV declares {bits}-bit PCM; only 16-bit is supported"
            );
            return Ok(WavFmt {
                sample_rate,
                channels,
            });
        }
        // RIFF chunks are word-aligned: skip the payload plus a pad byte.
        offset += 8 + size + (size & 1);
    }
    anyhow::bail!("WAV has no fmt chunk")
}

fn pcm_samples(body: &[u8]) -> Result<Vec<f32>> {
    let mut offset = 12usize;
    let mut samples: Vec<f32> = Vec::new();
    while offset + 8 <= body.len() {
        let id = &body[offset..offset + 4];
        let size = u32::from_le_bytes(body[offset + 4..offset + 8].try_into()?) as usize;
        if id == b"data" {
            let start = offset + 8;
            // A truncated reply must be an error, not silence: the
            // declared size and the actual bytes have to agree, or the
            // runner would announce and play a shorter sentence as if
            // it were complete.
            anyhow::ensure!(
                size > 0 && start + size <= body.len(),
                "WAV data chunk is empty or truncated"
            );
            let bytes = &body[start..start + size];
            // 16-bit little-endian interleave: no odd declared size is
            // accepted, because a half sample is a truncation, not
            // speech.
            anyhow::ensure!(
                bytes.len().is_multiple_of(2),
                "WAV data chunk ends mid-sample"
            );
            // 16-bit little-endian interleave. `as_chunks` keeps the
            // evenness guarantee visible and satisfies clippy's
            // chunks_exact_to_as_chunks (rust-lang/rust-clippy#14788).
            let (pairs, remainder) = bytes.as_chunks::<2>();
            debug_assert!(remainder.is_empty());
            for pair in pairs {
                let value = i16::from_le_bytes(*pair);
                samples.push(f32::from(value) / 32768.0);
            }
            return Ok(samples);
        }
        offset += 8 + size + (size & 1);
    }
    anyhow::bail!("WAV has no data chunk")
}

/// Resample a mono sample buffer from `from` Hz to `to` Hz with linear
/// interpolation.
///
/// The one place a rate mismatch could silently produce wrong-pitch
/// audio (the resampler is the only conversion between the 22050 Hz
/// voices and a 44100/48000 Hz output device), so the contract is
/// strict: identity returns the input unchanged, the output frame
/// count is `(frames * to / from)`, and every output position is
/// clamped-interpolated between its two nearest input frames.
///
/// Pure: no `cpal` types, no I/O. Unit-tested.
pub fn resample_linear(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() {
        return samples.to_vec();
    }
    let in_frames = samples.len();
    let out_frames = (in_frames as u64 * to as u64 / from as u64) as usize;
    let in_last = (in_frames - 1) as f64;
    (0..out_frames)
        .map(|out_index| {
            let position = out_index as f64 * from as f64 / to as f64;
            let low = position.floor();
            let low_index = low.min(in_last) as usize; // in case to < from
            let high_index = (low_index + 1).min(in_frames - 1);
            let frac = position - low;
            (samples[low_index] * (1.0 - frac as f32) + samples[high_index] * frac as f32)
                .clamp(-1.0, 1.0)
        })
        .collect()
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TTS_SAMPLE_RATE;

    /// Header builder mirroring how Piper writes WAVs: fmt comes first
    /// (16-byte payload, the four fields the parser reads are filled
    /// from the arguments), data last, and `extra: true` inserts a
    /// foreign chunk between them so the walker is proven not to
    /// assume fixed offsets.
    fn wav_body(
        format_tag: u16,
        sample_rate: u32,
        channels: u16,
        data: &[i16],
        extra: bool,
    ) -> Vec<u8> {
        let fmt_payload: [u8; 16] = [
            // format tag, channels,
            (format_tag & 0xff) as u8,
            (format_tag >> 8) as u8,
            (channels & 0xff) as u8,
            (channels >> 8) as u8,
            // sample rate in Hz,
            (sample_rate & 0xff) as u8,
            ((sample_rate >> 8) & 0xff) as u8,
            ((sample_rate >> 16) & 0xff) as u8,
            ((sample_rate >> 24) & 0xff) as u8,
            // byte rate and block align: not read by the parser,
            0,
            0,
            0,
            0,
            0,
            0,
            // bits per sample: 16,
            16,
            0,
        ];
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        // Placeholder master size; the parser ignores it, like a real
        // streaming writer.
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&(fmt_payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&fmt_payload);
        if extra {
            out.extend_from_slice(b"LIST");
            let note = b"INFOpiper";
            out.extend_from_slice(&(note.len() as u32).to_le_bytes());
            // Pad to even length after an odd payload.
            out.extend_from_slice(note);
            out.push(0);
        }
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32 * 2).to_le_bytes());
        for sample in data {
            out.extend_from_slice(&sample.to_le_bytes());
        }
        out
    }

    #[test]
    fn parses_a_realistic_wav_body() {
        let body = wav_body(
            1,
            TTS_SAMPLE_RATE,
            1,
            &[0, 16384, -16384, 32767, -32768],
            false,
        );
        let chunk = parse_wav_body(&body).expect("valid WAV must parse");
        assert_eq!(chunk.sample_rate, TTS_SAMPLE_RATE);
        assert_eq!(chunk.channels, 1);
        assert_eq!(chunk.samples.len(), 5);
        assert!((chunk.samples[1] - 0.5).abs() < 1e-3);
        assert!((chunk.samples[2] + 0.5).abs() < 1e-3);
        assert!((chunk.samples[4] + 1.0).abs() < 1e-3);
        assert!(chunk.is_final);
    }

    #[test]
    fn walks_chunks_even_when_the_order_is_not_fmt_data() {
        let body = wav_body(1, TTS_SAMPLE_RATE, 1, &[1024], true);
        let chunk = parse_wav_body(&body).expect("WAV with a LIST chunk must parse");
        assert_eq!(chunk.samples.len(), 1);
        assert!((chunk.samples[0] - 1024.0 / 32768.0).abs() < 1e-6);
    }

    #[test]
    fn rejects_an_empty_body() {
        assert!(parse_wav_body(&[]).is_err());
    }

    #[test]
    fn rejects_a_truncated_header() {
        assert!(parse_wav_body(b"RIFF....").is_err());
    }

    #[test]
    fn rejects_a_non_pcm_format_tag() {
        // IEEE float format (tag 3): refuse instead of misreading it.
        let body = wav_body(3, TTS_SAMPLE_RATE, 1, &[0], false);
        assert!(parse_wav_body(&body).is_err());
    }

    #[test]
    fn zero_declared_rate_or_channels_is_invalid() {
        let body = wav_body(1, 0, 1, &[0], false);
        assert!(parse_wav_body(&body).is_err());
        let body = wav_body(1, TTS_SAMPLE_RATE, 0, &[0], false);
        assert!(parse_wav_body(&body).is_err());
    }

    /// Piper truncation midpoint: the body cut right after the fmt
    /// chunk. Must refuse rather than loop or panic.
    #[test]
    fn rejects_a_data_chunk_that_never_starts() {
        let mut body = wav_body(1, TTS_SAMPLE_RATE, 1, &[1], false);
        body.truncate(44);
        assert!(parse_wav_body(&body).is_err());
    }

    #[test]
    fn ignores_a_trailing_odd_byte_in_the_data_chunk() {
        let mut body = wav_body(1, TTS_SAMPLE_RATE, 1, &[2048, -2048], false);
        body.push(0); // half of a third sample
        let chunk = parse_wav_body(&body).expect("odd trailing byte must be dropped, not fatal");
        assert_eq!(chunk.samples.len(), 2);
    }

    /// One second of speech at the native voice rate: the parsed chunk
    /// reports the duration the runner announces, straight from the
    /// header rate, not from a constant.
    #[test]
    fn one_second_at_22050_reports_1000ms() {
        let body = wav_body(1, TTS_SAMPLE_RATE, 1, &[0; 22_050], false);
        let chunk = parse_wav_body(&body).expect("a 1 s WAV must parse");
        assert_eq!(chunk.samples.len(), 22_050);
        assert_eq!(
            crate::synthesis::chunk_duration_ms(&chunk),
            1000,
            "22050 frames at 22050 Hz is one second"
        );
    }

    /// The walker reads bits-per-sample; a different depth must be
    /// refused, not decoded as two-byte samples.
    #[test]
    fn rejects_a_non_16_bit_depth() {
        let mut body = wav_body(1, TTS_SAMPLE_RATE, 1, &[0], false);
        // fmt payload starts at byte 20; bits-per-sample sits 14 bytes in.
        body[34] = 8;
        let err = parse_wav_body(&body).unwrap_err();
        assert!(
            err.to_string().contains("16-bit"),
            "error must name the depth, got: {err}"
        );
    }

    /// A body cut inside the declared fmt payload must be an error,
    /// not a slice out of bounds.
    #[test]
    fn rejects_a_fmt_payload_cut_midway() {
        let mut body = wav_body(1, TTS_SAMPLE_RATE, 1, &[1], false);
        body.truncate(24); // fmt id + size present, payload cut
        assert!(parse_wav_body(&body).is_err());
    }

    #[test]
    fn rejects_a_data_chunk_shorter_than_declared() {
        let mut body = wav_body(1, TTS_SAMPLE_RATE, 1, &[1, 2], false);
        // Declare four data bytes; deliver only the chunk header.
        body.truncate(body.len() - 2);
        assert!(parse_wav_body(&body).is_err());
    }

    /// A declared data size ending mid-sample is a truncation, not
    /// speech: refuse it instead of silently dropping the half sample.
    #[test]
    fn rejects_a_data_chunk_ending_mid_sample() {
        let mut body = wav_body(1, TTS_SAMPLE_RATE, 1, &[1, 2, 3], false);
        // Declare five data bytes instead of six (size's low byte sits
        // at offset 40; little-endian). The sixth byte stays in the
        // body beyond the declared chunk, so only the mid-sample rule
        // can fire.
        body[40] = 5;
        let err = parse_wav_body(&body).unwrap_err();
        assert!(
            err.to_string().contains("mid-sample"),
            "error must name the truncation, got: {err}"
        );
    }

    /// A liying rate makes `resample_linear` expand samples by
    /// rate/target: 1 Hz turns a small reply into gigabytes. Refuse
    /// rates outside the plausible speech range instead.
    #[test]
    fn rejects_a_sample_rate_outside_the_speech_range() {
        let body = wav_body(1, 1, 1, &[1, 2], false);
        let err = parse_wav_body(&body).unwrap_err();
        assert!(
            err.to_string().contains("sample rate"),
            "error must name the rate, got: {err}"
        );
        let body = wav_body(1, 500_000, 1, &[1], false);
        assert!(parse_wav_body(&body).is_err());
    }

    /// A stereo payload with a hanging sample folds into misaligned
    /// audio; refuse it instead of playing a half frame per channel.
    #[test]
    fn rejects_stereo_data_with_a_hanging_sample() {
        let body = wav_body(1, TTS_SAMPLE_RATE, 2, &[500], false);
        let err = parse_wav_body(&body).unwrap_err();
        assert!(
            err.to_string().contains("channel count"),
            "error must name the alignment, got: {err}"
        );
    }

    // ---- resample_linear -------------------------------------------------

    #[test]
    fn identity_resample_returns_the_input() {
        let samples = vec![0.1, -0.2, 0.3];
        assert_eq!(resample_linear(&samples, 48_000, 48_000), samples);
    }

    #[test]
    fn empty_input_stays_empty() {
        assert!(resample_linear(&[], TTS_SAMPLE_RATE, 48_000).is_empty());
    }

    #[test]
    fn single_sample_is_repeated_for_a_rate_increase() {
        // One input frame at 22050 -> 48000 is two frames by the ratio
        // formula; with no second frame to interpolate towards, both
        // carry the same value.
        assert_eq!(
            resample_linear(&[0.25], TTS_SAMPLE_RATE, 48_000),
            vec![0.25; 2]
        );
    }

    #[test]
    fn output_length_matches_the_rate_ratio() {
        // 10 frames at 22050 Hz -> 48000 Hz: ceil-ratio length contract
        // is floor(frames * to / from) = 21.
        let samples: Vec<f32> = vec![0.0; 10];
        assert_eq!(resample_linear(&samples, TTS_SAMPLE_RATE, 48_000).len(), 21);
    }

    #[test]
    fn resampling_interpolates_at_the_midpoint() {
        // Two frames 0..1 doubled in rate: 4 frames out, positions
        // 0.0, 0.5, 1.0, and a clamped repeat of the last.
        let out = resample_linear(&[0.0, 1.0], 2, 4);
        assert_eq!(out.len(), 4);
        assert!(out[0].abs() < 1e-6);
        assert!(
            (out[1] - 0.5).abs() < 1e-6,
            "midpoint must interpolate: {out:?}"
        );
        assert!((out[2] - 1.0).abs() < 1e-6);
        assert!(
            (out[3] - 1.0).abs() < 1e-6,
            "past the end clamps to the last frame"
        );
    }

    #[test]
    fn never_exceeds_full_scale() {
        let samples: Vec<f32> = vec![1.0; 4];
        let out = resample_linear(&samples, 8_000, 48_000);
        assert!(out.iter().all(|s| (-1.0..=1.0).contains(s)));
    }
}
