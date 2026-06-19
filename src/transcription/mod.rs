//! Transcription abstraction + mock + CPAL pipeline.
//!
//! `transcriber` defines the per-chunk contract (`TranscriptionResult`,
//! trait `Transcriber`). `mock` is the synchronous test/placeholder
//! impl. `pipeline` drives the CPAL capture loop and routes each chunk
//! through any `Transcriber`.

pub mod transcriber;
pub mod mock;
pub mod pipeline;
