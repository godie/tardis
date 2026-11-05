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

/// Append-only session history written by the Tauri shell: one JSON object
/// per stage output (transcript, rendered output) in the order they
/// happened, one per line. Relative to the working directory, like
/// [`LIVE_CHUNKS_DIR`].
pub const HISTORY_PATH: &str = "output/history.jsonl";

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

/// Base URL of the OpenAI-compatible LLM endpoint the real
/// `processor` talks to. Defaults to a local Ollama; a cloud endpoint
/// is the same code with a different base URL plus
/// `TARDIS_LLM_API_KEY`. Never contains the key.
pub const LLM_BASE_URL: &str = "http://localhost:11434/v1";

/// Model id sent as the `model` field. Must exist on the server above
/// (`ollama pull qwen2.5:7b-instruct` for the default).
pub const LLM_MODEL: &str = "qwen2.5:7b-instruct";

/// Per-request timeout, in seconds. One request is in flight at a time
/// (the loop is synchronous), so this is also the worst-case stall of
/// the capture thread when the model is cold.
pub const LLM_TIMEOUT_SECS: u64 = 20;

/// Cap on generated tokens per utterance. A translation never needs
/// more than the transcript itself; the cap bounds latency and cost.
pub const LLM_MAX_TOKENS: u32 = 256;

/// Sampling temperature. Low on purpose: translating is not a creative
/// task, and a warm model drifts on names and numbers.
pub const LLM_TEMPERATURE: f32 = 0.2;

/// Version of the process-stage prompt. Bumped whenever the prompt text
/// changes, so the change is visible in the prompt itself and in review.
pub const PROCESS_PROMPT_VERSION: u32 = 1;
