//! Synthesis stage: turn processed text into rendered audio.
//!
//! The stage contract is synchronous, object-safe, and free of CPAL
//! types, mirroring
//! [`crate::transcription::LocalTranscriptionProvider`] and
//! [`crate::processor::Processor`]: [`Synthesizer`] takes a
//! [`Speech`] (what to say, in which language) and returns an
//! [`AudioChunk`] (interleaved `f32`, sample rate, channel count,
//! finality). Nothing here knows about devices, streams, or playback;
//! that is [`crate::audio::player::Player`]'s job.
//!
//! [`chunk_duration_ms`] is the one shared pure helper: the mock and
//! any future real player both report the same duration for the same
//! chunk, so the UI's "Spoken · 1.2 s" line and the CLI's
//! `[chunk 1][speech es 1240ms]` line cannot disagree.
//!
//! Tests live in `#[cfg(test)] mod tests` per AGENTS.md conventions;
//! nothing in this module touches CPAL, the filesystem, or the clock.

pub mod mock;

pub use mock::MockSynthesizer;

/// What to say.
///
/// Built by the runner from the process stage's rendered text plus
/// the configured output language, so the synthesizer never has to
/// guess which language it is speaking.
#[derive(Debug, Clone, PartialEq)]
pub struct Speech {
    /// Text to render. Empty / whitespace-only means "nothing to
    /// say" and should yield `None`, not silence of zero length.
    pub text: String,
    /// Language code the text is written in (e.g. `"es"`). Real
    /// providers use it to pick a voice; the mock ignores it.
    pub language: String,
}

/// Rendered audio for one utterance.
///
/// Stage-agnostic on purpose: no `cpal` types in the contract, so the
/// same value can be produced by a mock, an HTTP TTS sidecar, or a
/// device, and consumed by a mock or a real player.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioChunk {
    /// Interleaved samples in `-1.0..=1.0`.
    pub samples: Vec<f32>,
    /// Sample rate in Hz. `0` is invalid and yields a `0 ms` duration
    /// (see [`chunk_duration_ms`]) rather than a division panic.
    pub sample_rate: u32,
    /// Interleaved channel count. `1` for every provider we plan to
    /// use; `0` is invalid and yields a `0 ms` duration.
    pub channels: u16,
    /// `true` for the terminal chunk of an utterance. Real streaming
    /// providers may emit partials with `false` first.
    pub is_final: bool,
}

/// Synchronous text-to-audio contract.
///
/// `synthesize` must be pure with respect to `&self`: the same
/// [`Speech`] and the same impl state must produce the same
/// [`AudioChunk`]. Returning `None` signals "nothing to say"
/// (empty text, or a provider that declined); the runner then emits
/// no speech event and does not call the player.
pub trait Synthesizer {
    fn synthesize(&self, speech: &Speech) -> Option<AudioChunk>;
}

/// Duration of `chunk` in whole milliseconds.
///
/// Computed from frame count (samples / channels) so a stereo buffer
/// does not report double its real length. Truncating division keeps
/// the result exact for the whole-millisecond chunks real providers
/// and the mock produce.
///
/// Degenerate inputs return `0` instead of panicking: `sample_rate`
/// or `channels` of `0`, or an empty sample buffer.
pub fn chunk_duration_ms(chunk: &AudioChunk) -> u64 {
    if chunk.sample_rate == 0 || chunk.channels == 0 {
        return 0;
    }
    let frames = chunk.samples.len() as u64 / chunk.channels as u64;
    frames * 1_000 / chunk.sample_rate as u64
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(samples: usize, sample_rate: u32, channels: u16) -> AudioChunk {
        AudioChunk {
            samples: vec![0.0; samples],
            sample_rate,
            channels,
            is_final: true,
        }
    }

    // ---- chunk_duration_ms ---------------------------------------------

    #[test]
    fn duration_of_mono_16k_quarter_second() {
        // 16 000 Hz * 0.25 s = 4 000 frames.
        assert_eq!(chunk_duration_ms(&chunk(4_000, 16_000, 1)), 250);
    }

    #[test]
    fn duration_of_mono_44k_one_second() {
        assert_eq!(chunk_duration_ms(&chunk(44_100, 44_100, 1)), 1_000);
    }

    #[test]
    fn duration_counts_frames_not_samples() {
        // 44 100 interleaved stereo samples == 22 050 frames == 0.5 s.
        assert_eq!(chunk_duration_ms(&chunk(44_100, 44_100, 2)), 500);
    }

    #[test]
    fn duration_of_empty_buffer_is_zero() {
        assert_eq!(chunk_duration_ms(&chunk(0, 16_000, 1)), 0);
    }

    #[test]
    fn duration_of_degenerate_sample_rate_is_zero() {
        assert_eq!(chunk_duration_ms(&chunk(1_000, 0, 1)), 0);
    }

    #[test]
    fn duration_of_degenerate_channel_count_is_zero() {
        assert_eq!(chunk_duration_ms(&chunk(1_000, 16_000, 0)), 0);
    }

    #[test]
    fn duration_truncates_sub_millisecond_chunks() {
        // 1 frame at 44 100 Hz is 0.02 ms; report 0, never 1.
        assert_eq!(chunk_duration_ms(&chunk(1, 44_100, 1)), 0);
    }
}
