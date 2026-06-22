//! AppState — pure ownership of the orchestration layer's mutable surface.
//!
//! Held by [`crate::app::service::AppService`]. Every mutator
//! updates state **and** returns the corresponding
//! [`crate::app::events::AppEvent`] so the caller can drain an
//! event stream without reading the state afterwards (or before).
//!
//! Methods are pure with respect to `&mut self`: no `&self`
//! calls return mutated values, no I/O happens.

use super::config::AppRuntimeConfig;
use super::events::{AppEvent, AppStatus, AppTranscriptEvent, AppTranslationEvent};

/// Owner of the orchestration layer's mutable surface.
///
/// Holds:
/// - the coarse-grained [`AppStatus`]
/// - the frozen-at-construction [`AppRuntimeConfig`]
/// - the most recent transcript / translation strings (so a
///   future UI shell can show "last heard" / "last translated"
///   at a glance without re-querying the event log).
#[derive(Debug, Clone, PartialEq)]
pub struct AppState {
    pub status: AppStatus,
    pub config: AppRuntimeConfig,
    pub last_transcript: Option<String>,
    pub last_translation: Option<String>,
}

impl AppState {
    /// Build a fresh state with [`AppStatus::Idle`] and `None`
    /// for both last-* fields. `config` is stored verbatim so
    /// subsequent reads return exactly what was validated by
    /// [`super::config::validate_runtime_config`].
    pub fn new(config: AppRuntimeConfig) -> Self {
        Self {
            status: AppStatus::Idle,
            config,
            last_transcript: None,
            last_translation: None,
        }
    }

    /// Set the status and return the corresponding
    /// `StatusChanged` event. Pure mutation.
    pub fn set_status(&mut self, status: AppStatus) -> AppEvent {
        self.status = status;
        AppEvent::StatusChanged(status)
    }

    /// Store the transcript's text in `last_transcript` and
    /// return the transcript event. The event is returned
    /// verbatim (no stripping, no truncation) so consumers can
    /// log the full provider-tagged payload.
    pub fn apply_transcript(&mut self, event: AppTranscriptEvent) -> AppEvent {
        self.last_transcript = Some(event.text.clone());
        AppEvent::Transcript(event)
    }

    /// Store the translation's text in `last_translation` and
    /// return the translation event. Pure mutation.
    pub fn apply_translation(&mut self, event: AppTranslationEvent) -> AppEvent {
        self.last_translation = Some(event.translated_text.clone());
        AppEvent::Translation(event)
    }
}

impl Default for AppState {
    fn default() -> Self {
        // `AppRuntimeConfig::default()` is documented to be valid
        // (see `config::tests::default_is_valid`), so building
        // default state is a no-fail operation.
        Self::new(AppRuntimeConfig::default())
    }
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> AppRuntimeConfig {
        AppRuntimeConfig::default()
    }

    // ---- new ----------------------------------------------------------

    #[test]
    fn new_starts_in_idle_status() {
        let state = AppState::new(sample_config());
        assert_eq!(state.status, AppStatus::Idle);
    }

    #[test]
    fn new_starts_with_no_last_transcript() {
        let state = AppState::new(sample_config());
        assert!(state.last_transcript.is_none());
    }

    #[test]
    fn new_starts_with_no_last_translation() {
        let state = AppState::new(sample_config());
        assert!(state.last_translation.is_none());
    }

    #[test]
    fn new_stores_config_verbatim() {
        let cfg = AppRuntimeConfig::default();
        let state = AppState::new(cfg.clone());
        assert_eq!(state.config, cfg);
    }

    // ---- Default ------------------------------------------------------

    #[test]
    fn default_is_idle_with_default_config() {
        let state = AppState::default();
        assert_eq!(state.status, AppStatus::Idle);
        assert_eq!(state.config, AppRuntimeConfig::default());
    }

    // ---- set_status ---------------------------------------------------

    #[test]
    fn set_status_updates_internal_status() {
        let mut state = AppState::new(sample_config());
        state.set_status(AppStatus::Listening);
        assert_eq!(state.status, AppStatus::Listening);
    }

    #[test]
    fn set_status_returns_status_changed_event() {
        let mut state = AppState::new(sample_config());
        let event = state.set_status(AppStatus::Stopped);
        assert_eq!(event, AppEvent::StatusChanged(AppStatus::Stopped));
    }

    // ---- apply_transcript --------------------------------------------

    #[test]
    fn apply_transcript_stores_text_in_last_transcript() {
        let mut state = AppState::new(sample_config());
        let event = AppTranscriptEvent {
            chunk_index: 1,
            text: "hello".to_string(),
            provider: "mock-local".to_string(),
            is_final: true,
        };
        state.apply_transcript(event.clone());
        assert_eq!(state.last_transcript.as_deref(), Some("hello"));
    }

    #[test]
    fn apply_transcript_returns_transcript_event() {
        let mut state = AppState::new(sample_config());
        let event = AppTranscriptEvent {
            chunk_index: 2,
            text: "world".to_string(),
            provider: "mock-local".to_string(),
            is_final: true,
        };
        let returned = state.apply_transcript(event.clone());
        assert_eq!(returned, AppEvent::Transcript(event));
    }

    // ---- apply_translation -------------------------------------------

    #[test]
    fn apply_translation_stores_text_in_last_translation() {
        let mut state = AppState::new(sample_config());
        let event = AppTranslationEvent {
            chunk_index: 1,
            source_text: "hello".to_string(),
            translated_text: "[mock es] mock translation: \"hello\"".to_string(),
            source_language: "en".to_string(),
            target_language: "es".to_string(),
            is_final: true,
        };
        state.apply_translation(event.clone());
        assert_eq!(
            state.last_translation.as_deref(),
            Some("[mock es] mock translation: \"hello\"")
        );
    }

    #[test]
    fn apply_translation_returns_translation_event() {
        let mut state = AppState::new(sample_config());
        let event = AppTranslationEvent {
            chunk_index: 3,
            source_text: "hi".to_string(),
            translated_text: "[mock es] hi".to_string(),
            source_language: "en".to_string(),
            target_language: "es".to_string(),
            is_final: true,
        };
        let returned = state.apply_translation(event.clone());
        assert_eq!(returned, AppEvent::Translation(event));
    }

    // ---- full sequence ------------------------------------------------

    #[test]
    fn full_start_to_stop_sequence() {
        let mut state = AppState::new(sample_config());

        // Idle -> Listening
        let e1 = state.set_status(AppStatus::Listening);
        assert_eq!(e1, AppEvent::StatusChanged(AppStatus::Listening));
        assert_eq!(state.status, AppStatus::Listening);

        // Apply transcript + translation events and verify each
        // mutator returns its own AppEvent variant while updating
        // the corresponding last_* field.
        let tr = state.apply_transcript(AppTranscriptEvent {
            chunk_index: 1,
            text: "transcribed".to_string(),
            provider: "mock-local".to_string(),
            is_final: true,
        });
        assert!(matches!(tr, AppEvent::Transcript(_)));
        assert_eq!(state.last_transcript.as_deref(), Some("transcribed"));

        let tl = state.apply_translation(AppTranslationEvent {
            chunk_index: 1,
            source_text: "transcribed".to_string(),
            translated_text: "[mock es] transcribed".to_string(),
            source_language: "en".to_string(),
            target_language: "es".to_string(),
            is_final: true,
        });
        assert!(matches!(tl, AppEvent::Translation(_)));
        assert_eq!(
            state.last_translation.as_deref(),
            Some("[mock es] transcribed")
        );

        // Listening -> Stopped
        let e2 = state.set_status(AppStatus::Stopped);
        assert_eq!(e2, AppEvent::StatusChanged(AppStatus::Stopped));
        assert_eq!(state.status, AppStatus::Stopped);
    }
}
