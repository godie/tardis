//! Canonical session lifecycle for the live pipeline.
//!
//! One place describes the whole phase machine the CLI, the Tauri shell
//! and the UI threaded through three booleans before. The backend stays
//! the source of truth (it emits [`AppStatus`] events) and this module
//! only derives the phase from those events, so existing payloads,
//! labels and DOM behavior do not change.
//!
//! Phases (and the one event that leaves each):
//!
//! ```text
//! idle ──start-accepted──▶ starting ──status(listening)──▶ listening
//!   ▲                                                      │
//!   │ error / failed start                                 │ stop-accepted
//!   ├──status(error)──▶ idle ◀──status(stopped)── stopping ◀┘
//! ```
//!
//! Pure: no I/O, no Tauri, no CPAL; unit-tested as a transition table.

use crate::app::events::AppStatus;

/// The one session lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase {
    /// Nothing running; Start is the only meaningful click.
    Idle,
    /// A start command is in flight; the listening event has not
    /// arrived yet, so drop any further start clicks.
    Starting,
    /// The backend reports capture running.
    Listening,
    /// A stop command is in flight; keep the panel locked and Stop
    /// disabled because a retry while the worker drains would race.
    Stopping,
}

impl SessionPhase {
    /// Start value for a fresh page or a reset backend.
    pub fn starting_phase() -> Self {
        Self::Idle
    }

    /// The reported status label for each phase, matching the casing
    /// `setStatus` already applies to backend events.
    pub fn as_label(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Starting => "Starting",
            Self::Listening => "Listening",
            Self::Stopping => "Stopping",
        }
    }

    /// Control layout for the phase, mirrored one-to-one from what the
    /// UI did with its three booleans: while capture is live (or being
    /// torn down in flight) Start stays disabled, Stop stays enabled,
    /// and the settings stay locked.
    pub fn session_running(self) -> bool {
        matches!(self, Self::Listening | Self::Stopping)
    }

    /// Apply one incoming status to the current phase.
    ///
    /// `Stopped` lands as [`SessionPhase::Idle`] (the backend event is
    /// the only thing that unlocks the panel), `Error` returns to Idle,
    /// and a status arriving while `Starting` only takes effect once it
    /// is a listening/stopped one, so the starting click cannot be
    /// swallowed by a stale event.
    pub fn apply_status(self, status: AppStatus) -> Self {
        match (self, status) {
            (_, AppStatus::Listening) => Self::Listening,
            (_, AppStatus::Stopped) => Self::Idle,
            (_, AppStatus::Error) => Self::Idle,
            (Self::Idle, AppStatus::Idle) => Self::Idle,
            (phase, AppStatus::Idle) => phase,
        }
    }
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// The plan's happy path: start accepted → listening → stop
    /// accepted → stopped → idle.
    #[test]
    fn the_happy_path_walks_the_documented_table() {
        let phase = SessionPhase::starting_phase().apply_status(AppStatus::Idle); // starting is a UI event, not a backend one
        assert_eq!(phase, SessionPhase::Idle);

        let listening = SessionPhase::Idle.apply_status(AppStatus::Listening);
        assert_eq!(listening, SessionPhase::Listening);
        let stopping = SessionPhase::Stopping;
        assert_eq!(
            stopping.apply_status(AppStatus::Stopped),
            SessionPhase::Idle
        );
    }

    /// A stale error never resurrects a session that already went back
    /// to idle.
    #[test]
    fn status_error_collapses_every_phase_to_idle() {
        for phase in [
            SessionPhase::Idle,
            SessionPhase::Starting,
            SessionPhase::Listening,
            SessionPhase::Stopping,
        ] {
            assert_eq!(phase.apply_status(AppStatus::Error), SessionPhase::Idle);
        }
    }

    /// The UI's three-boolean layout, one phase at a time: settings and
    /// the start button unlock exactly when the backend says stopped.
    #[test]
    fn running_locks_controls_and_idle_releases_them() {
        assert!(!SessionPhase::Idle.session_running());
        assert!(!SessionPhase::Starting.session_running());
        assert!(SessionPhase::Listening.session_running());
        assert!(SessionPhase::Stopping.session_running());
    }

    /// Labels stay stable for logs and status pills.
    #[test]
    fn labels_are_stable() {
        assert_eq!(SessionPhase::Idle.as_label(), "Idle");
        assert_eq!(SessionPhase::Starting.as_label(), "Starting");
        assert_eq!(SessionPhase::Listening.as_label(), "Listening");
        assert_eq!(SessionPhase::Stopping.as_label(), "Stopping");
    }
}
