//! Canonical session lifecycle for the live pipeline.
//!
//! One place describes the whole phase machine the CLI, the Tauri shell
//! and the UI threaded through three booleans before. The backend stays
//! the source of truth (it emits [`AppStatus`] events) and this module
//! only derives the phase from those events and the command outcomes, so
//! existing payloads, labels and DOM behavior do not change.
//!
//! Phases and the transitions that leave each:
//!
//! ```text
//! idle ──start-accepted──▶ starting ──status(listening)──▶ listening
//!   ▲                          │                             │
//!   │                          │ start-failed                │ stop-accepted
//!   │                          ▼                             ▼
//!   └──────────────────────── idle ◀──status(stopped)── stopping
//! ```
//!
//! `worker-exited` returns any phase to [`SessionPhase::Idle`], as do the
//! `stopped` and `error` statuses. A late `listening` status leaves
//! [`SessionPhase::Stopping`] alone; every other status input keeps the
//! [`SessionPhase::apply_status`] rules the UI mirrors. Every other pair
//! is an [`IllegalTransition`].
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

/// One input the session machine consumes.
///
/// The lifecycle accepts the outcomes of the two Tauri commands, the
/// backend status events, and the worker's own exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionInput {
    /// `start_live_transcription` returned success: a worker is running.
    StartAccepted,
    /// `start_live_transcription` returned an error.
    StartFailed,
    /// `stop_live_transcription` returned success: the worker is draining.
    StopAccepted,
    /// A backend status event.
    Status(AppStatus),
    /// The worker thread has exited, for any reason.
    WorkerExited,
}

/// A pair of phase and input the transition table does not allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IllegalTransition {
    /// Phase the machine was in.
    pub phase: SessionPhase,
    /// Input that has no transition out of `phase`.
    pub input: SessionInput,
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
    /// the only thing that unlocks the panel) and `Error` also returns
    /// to Idle, from any phase: a failed worker emits the terminal pair
    /// before it ever reports listening, so a settled start must not
    /// revive a session that already ended. An `Idle` report leaves a
    /// non-Idle phase alone, and a late `Listening` cannot pull a
    /// `Stopping` session back to listening, so stale events cannot
    /// swallow the starting click or reopen a stop that is draining.
    pub fn apply_status(self, status: AppStatus) -> Self {
        match (self, status) {
            (Self::Stopping, AppStatus::Listening) => Self::Stopping,
            (_, AppStatus::Listening) => Self::Listening,
            (_, AppStatus::Stopped) => Self::Idle,
            (_, AppStatus::Error) => Self::Idle,
            (Self::Idle, AppStatus::Idle) => Self::Idle,
            (phase, AppStatus::Idle) => phase,
        }
    }

    /// Apply one lifecycle input to the current phase.
    ///
    /// The command and worker inputs are strict: a start is accepted only
    /// from [`SessionPhase::Idle`], a failed start only leaves
    /// [`SessionPhase::Starting`], a stop is accepted only from
    /// [`SessionPhase::Listening`], and `WorkerExited` always lands on
    /// [`SessionPhase::Idle`]. Status inputs keep the
    /// [`SessionPhase::apply_status`] table above.
    pub fn apply(self, input: SessionInput) -> Result<Self, IllegalTransition> {
        match (self, input) {
            (Self::Idle, SessionInput::StartAccepted) => Ok(Self::Starting),
            (Self::Starting, SessionInput::StartFailed) => Ok(Self::Idle),
            (Self::Listening, SessionInput::StopAccepted) => Ok(Self::Stopping),
            (_, SessionInput::WorkerExited) => Ok(Self::Idle),
            (phase, SessionInput::Status(status)) => Ok(phase.apply_status(status)),
            (phase, input) => Err(IllegalTransition { phase, input }),
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

    /// The whole table: every phase crossed with every input. `None` marks
    /// the illegal pairs, which must carry the phase and input they came
    /// from.
    #[test]
    fn the_full_transition_table_matches_the_plan() {
        use AppStatus::{
            Error, Idle as StatusIdle, Listening as StatusListening, Stopped as StatusStopped,
        };
        use SessionInput::{StartAccepted, StartFailed, Status, StopAccepted, WorkerExited};

        let table: &[(SessionPhase, SessionInput, Option<SessionPhase>)] = &[
            // Idle
            (
                SessionPhase::Idle,
                StartAccepted,
                Some(SessionPhase::Starting),
            ),
            (SessionPhase::Idle, StartFailed, None),
            (SessionPhase::Idle, StopAccepted, None),
            (
                SessionPhase::Idle,
                Status(StatusIdle),
                Some(SessionPhase::Idle),
            ),
            (
                SessionPhase::Idle,
                Status(StatusListening),
                Some(SessionPhase::Listening),
            ),
            (
                SessionPhase::Idle,
                Status(StatusStopped),
                Some(SessionPhase::Idle),
            ),
            (SessionPhase::Idle, Status(Error), Some(SessionPhase::Idle)),
            (SessionPhase::Idle, WorkerExited, Some(SessionPhase::Idle)),
            // Starting
            (SessionPhase::Starting, StartAccepted, None),
            (
                SessionPhase::Starting,
                StartFailed,
                Some(SessionPhase::Idle),
            ),
            (SessionPhase::Starting, StopAccepted, None),
            (
                SessionPhase::Starting,
                Status(StatusIdle),
                Some(SessionPhase::Starting),
            ),
            (
                SessionPhase::Starting,
                Status(StatusListening),
                Some(SessionPhase::Listening),
            ),
            (
                SessionPhase::Starting,
                Status(StatusStopped),
                Some(SessionPhase::Idle),
            ),
            (
                SessionPhase::Starting,
                Status(Error),
                Some(SessionPhase::Idle),
            ),
            (
                SessionPhase::Starting,
                WorkerExited,
                Some(SessionPhase::Idle),
            ),
            // Listening
            (SessionPhase::Listening, StartAccepted, None),
            (SessionPhase::Listening, StartFailed, None),
            (
                SessionPhase::Listening,
                StopAccepted,
                Some(SessionPhase::Stopping),
            ),
            (
                SessionPhase::Listening,
                Status(StatusIdle),
                Some(SessionPhase::Listening),
            ),
            (
                SessionPhase::Listening,
                Status(StatusListening),
                Some(SessionPhase::Listening),
            ),
            (
                SessionPhase::Listening,
                Status(StatusStopped),
                Some(SessionPhase::Idle),
            ),
            (
                SessionPhase::Listening,
                Status(Error),
                Some(SessionPhase::Idle),
            ),
            (
                SessionPhase::Listening,
                WorkerExited,
                Some(SessionPhase::Idle),
            ),
            // Stopping
            (SessionPhase::Stopping, StartAccepted, None),
            (SessionPhase::Stopping, StartFailed, None),
            (SessionPhase::Stopping, StopAccepted, None),
            (
                SessionPhase::Stopping,
                Status(StatusIdle),
                Some(SessionPhase::Stopping),
            ),
            (
                SessionPhase::Stopping,
                Status(StatusListening),
                Some(SessionPhase::Stopping),
            ),
            (
                SessionPhase::Stopping,
                Status(StatusStopped),
                Some(SessionPhase::Idle),
            ),
            (
                SessionPhase::Stopping,
                Status(Error),
                Some(SessionPhase::Idle),
            ),
            (
                SessionPhase::Stopping,
                WorkerExited,
                Some(SessionPhase::Idle),
            ),
        ];

        for &(phase, input, next) in table {
            let expected = match next {
                Some(next) => Ok(next),
                None => Err(IllegalTransition { phase, input }),
            };
            assert_eq!(phase.apply(input), expected, "{phase:?} + {input:?}");
        }
    }

    /// The worker's exit is the one universal input: it ends every phase.
    #[test]
    fn worker_exited_takes_any_phase_to_idle() {
        for phase in [
            SessionPhase::Idle,
            SessionPhase::Starting,
            SessionPhase::Listening,
            SessionPhase::Stopping,
        ] {
            assert_eq!(
                phase.apply(SessionInput::WorkerExited),
                Ok(SessionPhase::Idle)
            );
        }
    }

    /// A stop owns the phase until the backend reports the worker gone; a
    /// late `listening` must not reopen it, matching the UI's guard.
    #[test]
    fn a_late_listening_does_not_reopen_a_stopping_session() {
        assert_eq!(
            SessionPhase::Stopping.apply(SessionInput::Status(AppStatus::Listening)),
            Ok(SessionPhase::Stopping)
        );
    }
}
