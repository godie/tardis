//! Public library surface for the `tardis` core.
//!
//! Exposes the audio capture / volume / activity helpers, the
//! centralised configuration constants, and the transcription +
//! translation abstractions that the CLI binary ([`tardis::main`])
//! and the desktop Tauri shell
//! (`tardis-ui-shell`) both depend on, so the same Rust logic powers
//! both surfaces without duplication.
//!
//! Modules are declared `pub` so the Tauri shell (`src-tauri`) can
//! reach the provider-agnostic entry point
//! [`transcription::build_provider`] and the
//! [`config::LOCAL_WHISPER_*`] constants directly.

pub mod audio;
pub mod config;
pub mod transcription;
pub mod translation;
