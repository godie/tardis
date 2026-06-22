//! `AppService` — orchestrator for app-level pure-text flows.
//!
//! The intended wiring is:
//!
//! `CLI command or Tauri command`
//!   `-> AppService / backend orchestration layer`
//!   `-> audio / transcription / translation modules`
//!   `-> AppEvent outputs`
//!
//! Today the service is intentionally minimal and pure: it
//! orchestrates **text-input** flows only, holds no CPAL
//! stream, no Docker handle, no HTTP client, and no filesystem
//! handle. The `MockTranscriber` family of tests on the existing
//! pipelines is not exercised here because `run_mock_text_flow`
//! takes a transcript string directly (no chunk samples). Future
//! live-capture commands will reuse the same `AppEvent` shape
//! through a richer service implementation.
//!
//! The service is intentionally **not** `Clone`: it is meant to
//! sit behind a `Mutex` in a Tauri state container. A copyable
//! service would invite state-split bugs where two callers
//! observe divergent `last_transcript` values.

use super::config::{AppRuntimeConfig, validate_runtime_config};
use super::events::{AppEvent, AppStatus, AppTranscriptEvent, AppTranslationEvent};
use super::state::AppState;
use crate::translation::mock::MockTranslator;
use crate::translation::translator::Translator;

/// `run_mock_text_flow` is not driven by CPAL chunks, so it
/// synthesises a single transcript at chunk index `1`. Kept as
/// a `const` so docs and tests can refer to the exact index the
/// synthetic flow emits and so a future live capture layer can
/// continue the same numbering when wiring chunks 1..N.
const SYNTHETIC_CHUNK_INDEX: usize = 1;

/// App-facing orchestration entry point.
///
/// Holds an [`AppState`] and routes pure-text flow calls through
/// the existing [`crate::translation::mock::MockTranslator`].
///
/// Derives `Debug` (so tests can use `Result::unwrap_err()`
/// freely on the [`Self::new`] validation path) but does
/// **not** derive `Clone` — see the module-level note on
/// Tauri state-split risk.
#[derive(Debug)]
pub struct AppService {
    state: AppState,
}

impl AppService {
    /// Construct an [`AppService`] from an [`AppRuntimeConfig`].
    ///
    /// Runs [`validate_runtime_config`] before constructing
    /// state so any downstream consumer can rely on the config
    /// in `state.config()` being valid.
    pub fn new(config: AppRuntimeConfig) -> Result<Self, String> {
        validate_runtime_config(&config)?;
        Ok(Self {
            state: AppState::new(config),
        })
    }

    /// Read-only access to the inner [`AppState`].
    ///
    /// `AppState` is `Clone` so callers that need to retain a
    /// snapshot (for tests, for UI tickers) can `.clone()` it
    /// without going through `AppService`.
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Transition `status` to [`AppStatus::Listening`] and
    /// return the resulting [`AppEvent::StatusChanged`] event
    /// wrapped in a single-element [`Vec`].
    ///
    /// Always emits exactly one event (no idempotency check); the
    /// spec mandates "changes status to Listening and returns
    /// one StatusChanged event". The `Vec` return shape mirrors
    /// `run_mock_text_flow` so callers can unify event
    /// consumption through a single channel.
    pub fn start_listening_mock(&mut self) -> Vec<AppEvent> {
        vec![self.state.set_status(AppStatus::Listening)]
    }

    /// Transition `status` to [`AppStatus::Stopped`] and
    /// return the resulting [`AppEvent::StatusChanged`] event
    /// wrapped in a single-element [`Vec`]. Mirror of
    /// [`Self::start_listening_mock`].
    pub fn stop_listening(&mut self) -> Vec<AppEvent> {
        vec![self.state.set_status(AppStatus::Stopped)]
    }

    /// Run a pure-text mock flow:
    ///
    /// 1. If `transcript_text` is empty or whitespace-only,
    ///    **silently skip** — return an empty `Vec<AppEvent>`
    ///    and leave state untouched. This mirrors
    ///    [`MockTranslator`]'s "skip on empty" philosophy and
    ///    avoids forcing future UI shells to render
    ///    "empty transcript" toasts every capture tick.
    /// 2. Otherwise, build an [`AppTranscriptEvent`] with the
    ///    active provider name (from `state.config`) and store
    ///    + emit it via [`AppState::apply_transcript`].
    /// 3. Route the **same `transcript_text`** through
    ///    [`MockTranslator`] with the configured language pair;
    ///    if the translator returns a translation, store + emit
    ///    it via [`AppState::apply_translation`].
    /// 4. Return the 1-or-2 emitted events in chronological
    ///    order (transcript first, translation second).
    ///
    /// Pure with respect to `&mut self` — no I/O, no CPAL, no
    /// network, no filesystem.
    pub fn run_mock_text_flow(&mut self, transcript_text: &str) -> Vec<AppEvent> {
        if transcript_text.trim().is_empty() {
            return Vec::new();
        }

        let mut events = Vec::with_capacity(2);

        // 1. Synthesise + emit transcript.
        let transcript_event = AppTranscriptEvent {
            chunk_index: SYNTHETIC_CHUNK_INDEX,
            text: transcript_text.to_string(),
            provider: self.state.config.transcription_provider.clone(),
            is_final: true,
        };
        events.push(self.state.apply_transcript(transcript_event));

        // 2. Route through MockTranslator and emit translation
        //    (if any).
        let provider = MockTranslator::new();
        if let Some(translation) = provider.translate_text(
            transcript_text,
            &self.state.config.source_language,
            &self.state.config.target_language,
        ) {
            let translation_event = AppTranslationEvent {
                chunk_index: SYNTHETIC_CHUNK_INDEX,
                source_text: translation.source_text.clone(),
                translated_text: translation.translated_text.clone(),
                source_language: translation.source_language.clone(),
                target_language: translation.target_language.clone(),
                is_final: translation.is_final,
            };
            events.push(self.state.apply_translation(translation_event));
        }

        events
    }
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::events::status_label;

    fn service_with_default_config() -> AppService {
        AppService::new(AppRuntimeConfig::default()).expect("default config must validate")
    }

    // ---- new ----------------------------------------------------------

    #[test]
    fn new_rejects_invalid_config_with_empty_provider() {
        let mut cfg = AppRuntimeConfig::default();
        cfg.transcription_provider = String::new();
        let err = AppService::new(cfg).unwrap_err();
        assert!(
            err.contains("transcription_provider"),
            "error should name the offending field, got: {err}"
        );
    }

    #[test]
    fn new_rejects_zero_chunk_duration() {
        let mut cfg = AppRuntimeConfig::default();
        cfg.chunk_duration_ms = 0;
        let err = AppService::new(cfg).unwrap_err();
        assert!(
            err.contains("chunk_duration_ms"),
            "expected chunk_duration error, got: {err}"
        );
    }

    #[test]
    fn new_rejects_negative_threshold() {
        let mut cfg = AppRuntimeConfig::default();
        cfg.volume_threshold = -0.1;
        let err = AppService::new(cfg).unwrap_err();
        assert!(
            err.contains("volume_threshold"),
            "error should name threshold, got: {err}"
        );
    }

    #[test]
    fn new_accepts_default_config() {
        let service = AppService::new(AppRuntimeConfig::default()).expect("default must validate");
        assert_eq!(service.state().status, AppStatus::Idle);
    }

    #[test]
    fn new_initial_state_is_idle_and_empty() {
        let service = service_with_default_config();
        let state = service.state();
        assert_eq!(state.status, AppStatus::Idle);
        assert!(state.last_transcript.is_none());
        assert!(state.last_translation.is_none());
    }

    // ---- start_listening_mock ----------------------------------------

    #[test]
    fn start_listening_mock_transitions_to_listening() {
        let mut service = service_with_default_config();
        let events = service.start_listening_mock();
        assert_eq!(events, vec![AppEvent::StatusChanged(AppStatus::Listening)]);
        assert_eq!(service.state().status, AppStatus::Listening);
    }

    // ---- stop_listening -----------------------------------------------

    #[test]
    fn stop_listening_transitions_to_stopped() {
        let mut service = service_with_default_config();
        service.start_listening_mock();
        let events = service.stop_listening();
        assert_eq!(events, vec![AppEvent::StatusChanged(AppStatus::Stopped)]);
        assert_eq!(service.state().status, AppStatus::Stopped);
    }

    // ---- run_mock_text_flow: non-empty input -------------------------

    #[test]
    fn run_mock_text_flow_emits_two_events_for_non_empty_text() {
        let mut service = service_with_default_config();
        let events = service.run_mock_text_flow("mock transcript: speech detected");
        assert_eq!(events.len(), 2, "expected transcript + translation events");

        match &events[0] {
            AppEvent::Transcript(t) => {
                assert_eq!(t.chunk_index, SYNTHETIC_CHUNK_INDEX);
                assert_eq!(t.text, "mock transcript: speech detected");
                assert!(t.is_final);
            }
            other => panic!("event[0] must be Transcript, got: {other:?}"),
        }
        match &events[1] {
            AppEvent::Translation(t) => {
                assert_eq!(t.chunk_index, SYNTHETIC_CHUNK_INDEX);
                assert_eq!(t.source_text, "mock transcript: speech detected");
                assert!(t.translated_text.contains("mock"));
                assert!(t.translated_text.contains("es"));
                assert!(t.is_final);
            }
            other => panic!("event[1] must be Translation, got: {other:?}"),
        }
    }

    #[test]
    fn run_mock_text_flow_updates_last_transcript() {
        let mut service = service_with_default_config();
        service.run_mock_text_flow("hello");
        assert_eq!(service.state().last_transcript.as_deref(), Some("hello"));
    }

    #[test]
    fn run_mock_text_flow_updates_last_translation() {
        let mut service = service_with_default_config();
        service.run_mock_text_flow("hello");
        let last_translation = service
            .state()
            .last_translation
            .as_deref()
            .expect("last_translation must be set after run_mock_text_flow");
        assert!(last_translation.contains("hello"));
        assert!(last_translation.contains("es"));
    }

    #[test]
    fn run_mock_text_flow_uses_configured_source_and_target_languages() {
        let cfg = AppRuntimeConfig {
            source_language: "en".to_string(),
            target_language: "fr".to_string(),
            ..AppRuntimeConfig::default()
        };
        let mut service = AppService::new(cfg).expect("config must validate");
        let events = service.run_mock_text_flow("hello");
        let translation = match &events[1] {
            AppEvent::Translation(t) => t,
            other => panic!("event[1] must be Translation, got: {other:?}"),
        };
        assert_eq!(translation.source_language, "en");
        assert_eq!(translation.target_language, "fr");
    }

    #[test]
    fn run_mock_text_flow_uses_configured_provider_on_transcript_event() {
        let cfg = AppRuntimeConfig {
            transcription_provider: "local-whisper".to_string(),
            ..AppRuntimeConfig::default()
        };
        let mut service = AppService::new(cfg).expect("config must validate");
        let events = service.run_mock_text_flow("hello");
        let transcript = match &events[0] {
            AppEvent::Transcript(t) => t,
            other => panic!("event[0] must be Transcript, got: {other:?}"),
        };
        assert_eq!(transcript.provider, "local-whisper");
    }

    // ---- run_mock_text_flow: empty / whitespace -----------------------

    #[test]
    fn run_mock_text_flow_returns_empty_vec_for_empty_text() {
        let mut service = service_with_default_config();
        let events = service.run_mock_text_flow("");
        assert!(
            events.is_empty(),
            "empty text must produce no events; got: {events:?}"
        );
        // State must remain untouched so the empty-input path
        // is observably a no-op.
        assert!(service.state().last_transcript.is_none());
        assert!(service.state().last_translation.is_none());
        assert_eq!(service.state().status, AppStatus::Idle);
    }

    #[test]
    fn run_mock_text_flow_returns_empty_vec_for_whitespace_text() {
        let mut service = service_with_default_config();
        let events = service.run_mock_text_flow("\t  \n");
        assert!(events.is_empty());
        assert!(service.state().last_transcript.is_none());
        assert!(service.state().last_translation.is_none());
    }

    // ---- full lifecycle through AppService ----------------------------

    #[test]
    fn full_lifecycle_emits_expected_events() {
        let mut service = service_with_default_config();

        let e1 = service.start_listening_mock();
        assert_eq!(e1, vec![AppEvent::StatusChanged(AppStatus::Listening)]);

        let e2 = service.run_mock_text_flow("hello");
        assert_eq!(e2.len(), 2);
        assert!(matches!(e2[0], AppEvent::Transcript(_)));
        assert!(matches!(e2[1], AppEvent::Translation(_)));

        let e3 = service.stop_listening();
        assert_eq!(e3, vec![AppEvent::StatusChanged(AppStatus::Stopped)]);

        assert_eq!(status_label(service.state().status), "Stopped");
    }

    // `AppErrorEvent` is not emitted by the service today; it is
    // exercised directly in `events::tests::error_events_are_comparable`.
    // No sentinel needed here.
}
