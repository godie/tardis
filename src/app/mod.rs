//! App-facing orchestration layer.
//!
//! The target shape is:
//!
//! `CLI command or Tauri command`
//!   `-> AppService / backend orchestration layer`
//!   `-> audio/transcription/translation modules`
//!   `-> AppEvent outputs`
//!
//! Today only the synchronous, mock-only surface is wired
//! ([`service::AppService::start_listening_mock`],
//! [`service::AppService::stop_listening`],
//! [`service::AppService::run_mock_text_flow`]). Future live
//! capture and Tauri integration consume the same
//! [`events::AppEvent`] stream.
//!
//! The layer holds **no** CPAL stream, no Docker conduit, no HTTP
//! client, and no filesystem handle today; pure-text flows only.
//! Tests live next to the code in `#[cfg(test)] mod tests` per
//! AGENTS.md conventions.

pub mod config;
pub mod events;
pub mod live_events;
pub mod service;
pub mod settings_store;
pub mod state;
pub mod ui_events;
