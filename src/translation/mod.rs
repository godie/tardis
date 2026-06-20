//! Translation abstraction + mock + CPAL pipeline.
//!
//! `translator` defines the per-call contract (`TranslationResult`,
//! trait `Translator`). `mock` is the synchronous test/placeholder
//! impl. `pipeline` drives the CPAL capture loop, routes each chunk
//! through any `Transcriber` first, and — when speech is detected —
//! forwards the resulting text through any `Translator` to emit a
//! translation line.
//!
//! Both the `Transcriber` and `Translator` traits stay audio-agnostic;
//! only `pipeline` imports CPAL.

pub mod translator;
pub mod mock;
pub mod pipeline;
