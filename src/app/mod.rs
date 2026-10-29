//! App-facing orchestration layer.
//!
//! The target shape is:
//!
//! `CLI command or Tauri command`
//!   `-> AppService / backend orchestration layer`
//!   `-> audio/transcription/translation modules`
//!   `-> AppEvent outputs`
//!
//! Both frontends consume the layer today. The mock flows go through
//! [`service::AppService`] ([`service::AppService::start_listening_mock`],
//! [`service::AppService::stop_listening`],
//! [`service::AppService::run_mock_text_flow`]); the live path builds the
//! same [`events::AppEvent`] values in [`live_events`], and the Tauri shell
//! maps them to serializable payloads in [`ui_events`]. [`settings_store`]
//! persists [`config::AppRuntimeConfig`] to the OS config dir.
//!
//! The layer holds **no** CPAL stream, no Docker conduit, and no HTTP
//! client: audio devices and providers are injected by the CLI and
//! `src-tauri`. Tests live next to the code in `#[cfg(test)] mod tests`
//! per AGENTS.md conventions.

pub mod config;
pub mod events;
pub mod live_events;
pub mod service;
pub mod settings_store;
pub mod state;
pub mod ui_events;
