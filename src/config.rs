//! Centralized reusable configuration values.
//!
//! These constants replace hardcoded literals that were previously
//! scattered across `src/audio/`, `src/transcription/`, and
//! `src/translation/`. Their values are exercised by the existing unit
//! tests of the modules that consume them; constants themselves are
//! not unit-tested because they have no behavior beyond their values.
//!
//! Adding CLI flags, environment variables, or a config file layer is
//! intentionally out of scope until the mock pipelines are validated.

/// Default short-capture window, in seconds, used by `mic-5s` and
/// `record-5s`.
pub const DEFAULT_SHORT_CAPTURE_SECONDS: u64 = 5;

/// Default pipeline run length, in seconds, used by `chunk-test`,
/// `mock-transcribe`, `save-chunks-test`, and `mock-translate`.
pub const DEFAULT_PIPELINE_TEST_SECONDS: u64 = 10;

/// Default per-chunk length, in milliseconds, for all
/// pipeline-style commands (chunks are 1 s by default).
pub const DEFAULT_CHUNK_DURATION_MS: u64 = 1000;

/// Speech-vs-silence decision threshold on the same scale as
/// `audio::volume::calculate_average_volume`'s output. Used by the
/// mic activity log, both mock pipelines, and the file-based
/// transcriber.
pub const DEFAULT_VOLUME_THRESHOLD: f32 = 0.01;

/// Default non-speech time tolerated inside one utterance, in
/// milliseconds. Long enough to survive a pause between words in a slow
/// sentence, short enough that a finished turn is not kept open.
pub const DEFAULT_HANGOVER_MS: u64 = 400;

/// Default shortest burst kept as an utterance, in milliseconds. Anything
/// shorter is treated as noise: no event, no WAV, no provider call.
pub const DEFAULT_MIN_SPEECH_MS: u64 = 200;

/// Default burst cap, in milliseconds. A speaker who never pauses is
/// flushed at this length so the capture buffer cannot grow without bound.
pub const DEFAULT_MAX_UTTERANCE_MS: u64 = 15_000;

/// Root directory the CLI writes all artefacts (WAV recordings, per-chunk
/// files) into. Reserved for future directory listings / bulk-cleanup
/// commands; [`DEFAULT_MIC_RECORDING_PATH`] and [`DEFAULT_CHUNKS_DIR`]
/// currently duplicate the prefix inline because [`concat!`] only accepts
/// string literals, not `const &str` arguments.
#[allow(dead_code)]
pub const OUTPUT_DIR: &str = "output";

/// Default WAV destination for `record-5s`.
pub const DEFAULT_MIC_RECORDING_PATH: &str = "output/mic_test.wav";

/// Default directory used by `save-chunks-test` to write one WAV per
/// captured chunk.
pub const DEFAULT_CHUNKS_DIR: &str = "output/chunks";

/// Default directory used by `live-local-transcribe` to write one WAV
/// per speech-like chunk before sending it to a
/// [`crate::transcription::LocalTranscriptionProvider`].
pub const LIVE_CHUNKS_DIR: &str = "output/live_chunks";

/// Default source language code passed to translation pipelines
/// (BCP-47-ish, lowercase).
pub const DEFAULT_SOURCE_LANGUAGE: &str = "en";

/// Default target language code passed to translation pipelines
/// (BCP-47-ish, lowercase).
pub const DEFAULT_TARGET_LANGUAGE: &str = "es";

/// Bits per sample used by every 16-bit Int PCM WAV writer in this
/// crate (both `audio::recorder` and `audio::chunk_recorder`).
pub const WAV_BITS_PER_SAMPLE: u16 = 16;

/// Base URL of the local faster-whisper Docker server. Matches
/// `docker/faster-whisper/docker-compose.yml`'s `127.0.0.1:8000:8000`
/// port binding; audio never leaves the loopback interface.
pub const LOCAL_WHISPER_BASE_URL: &str = "http://localhost:8000";

/// Default faster-whisper model id passed as the multipart `model`
/// field to the local server. Must match the value the Docker
/// container is configured to load (see `WHISPER_MODEL` in
/// `docker/faster-whisper/docker-compose.yml`).
pub const LOCAL_WHISPER_MODEL: &str = "base";

/// Default language code passed as the multipart `language` field
/// to the local faster-whisper container.
pub const LOCAL_WHISPER_LANGUAGE: &str = "en";
