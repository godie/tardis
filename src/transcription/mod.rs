//! Mock transcription pipeline.
//!
//! `mock` holds the pure per-chunk classifier; `pipeline` drives the CPAL
//! capture loop and routes each chunk through it.

pub mod mock;
pub mod pipeline;
