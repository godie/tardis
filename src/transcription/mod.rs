//! Transcription abstraction + mock + CPAL pipeline.
//!
//! `transcriber` defines the per-chunk contract (`TranscriptionResult`,
//! trait `Transcriber`). `mock` is the synchronous test/placeholder
//! impl. `pipeline` drives the CPAL capture loop and routes each chunk
//! through any `Transcriber`. `file_pipeline` reads a previously-saved
//! WAV from disk and routes it through the same `Transcriber` without
//! re-running the microphone capture.

pub mod file_pipeline;
pub mod transcriber;
pub mod mock;
pub mod pipeline;
