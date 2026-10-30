//! Half-duplex gate: never listen while we speak.
//!
//! Pausing the drain is not enough. The capture stream keeps filling its
//! ring buffer while the player is talking, so the moment draining resumes
//! the pipeline would transcribe our own synthesized voice. Two rules fix
//! that. Both are expressed here as pure state.
//!
//! 1. **While speaking, captured frames are discarded** and the buffer is
//!    cleared when playback ends, so nothing from the output side survives
//!    into the next utterance.
//! 2. **A short tail after playback is discarded too** (250 ms by default),
//!    covering room reverb and the audio device's own output latency.
//!
//! The gate owns no clock: the caller advances it by however many
//! milliseconds of audio it captured or dropped, which keeps the whole
//! decision unit-testable (DEVELOPMENT_RULES rule 1) and the CPAL wiring on
//! the manual path (rule 2).

/// Default reverb tail discarded after playback ends, in milliseconds.
///
/// 250 ms covers a typical small room plus USB-interface output latency;
/// it is deliberately short enough that the next question is not clipped.
pub const DEFAULT_TAIL_MS: u64 = 250;

/// Pure half-duplex state machine.
///
/// Ordering, as the runner uses it:
///
/// ```text
/// begin_speaking()  -> play_chunk() (blocking) -> end_speaking()
/// discard pending capture, advance() the tail to zero -> drain normally
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HalfDuplexGate {
    speaking: bool,
    tail_ms: u64,
    tail_remaining_ms: u64,
}

impl HalfDuplexGate {
    /// Build a gate with an explicit tail length. `0` disables the tail,
    /// which the tests use to isolate the speaking rule.
    pub fn new(tail_ms: u64) -> Self {
        Self {
            speaking: false,
            tail_ms,
            tail_remaining_ms: 0,
        }
    }

    /// Build a gate with [`DEFAULT_TAIL_MS`].
    pub fn with_default_tail() -> Self {
        Self::new(DEFAULT_TAIL_MS)
    }

    /// Enter the speaking state. Returns `false` when already speaking, so
    /// a nested playback cannot silently reset the tail accounting.
    pub fn begin_speaking(&mut self) -> bool {
        if self.speaking {
            return false;
        }
        self.speaking = true;
        // A new utterance cancels any leftover tail: we are the source now.
        self.tail_remaining_ms = 0;
        true
    }

    /// Leave the speaking state and start the reverb tail.
    pub fn end_speaking(&mut self) {
        self.speaking = false;
        self.tail_remaining_ms = self.tail_ms;
    }

    /// Is the player currently speaking?
    pub fn is_speaking(&self) -> bool {
        self.speaking
    }

    /// Milliseconds of tail still to discard.
    pub fn tail_remaining_ms(&self) -> u64 {
        self.tail_remaining_ms
    }

    /// May the runner hand captured frames to the rest of the pipeline?
    ///
    /// `false` while speaking and while the post-playback tail is running.
    pub fn drain_allowed(&self) -> bool {
        !self.speaking && self.tail_remaining_ms == 0
    }

    /// Account for `captured_ms` of capture time that was dropped.
    ///
    /// Only the tail is time-based: while speaking, time passing does not
    /// unblock anything (playback itself ends the speaking state).
    pub fn advance(&mut self, captured_ms: u64) {
        if !self.speaking {
            self.tail_remaining_ms = self.tail_remaining_ms.saturating_sub(captured_ms);
        }
    }
}

impl Default for HalfDuplexGate {
    fn default() -> Self {
        Self::with_default_tail()
    }
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_gate_allows_draining() {
        let gate = HalfDuplexGate::with_default_tail();
        assert!(gate.drain_allowed());
        assert!(!gate.is_speaking());
        assert_eq!(gate.tail_remaining_ms(), 0);
    }

    #[test]
    fn speaking_blocks_draining() {
        let mut gate = HalfDuplexGate::with_default_tail();
        assert!(gate.begin_speaking());
        assert!(gate.is_speaking());
        assert!(!gate.drain_allowed());
    }

    #[test]
    fn begin_speaking_twice_reports_the_nested_call() {
        let mut gate = HalfDuplexGate::with_default_tail();
        assert!(gate.begin_speaking());
        assert!(!gate.begin_speaking(), "already speaking must return false");
    }

    #[test]
    fn tail_blocks_draining_after_playback() {
        let mut gate = HalfDuplexGate::with_default_tail();
        gate.begin_speaking();
        gate.end_speaking();
        assert!(!gate.is_speaking());
        assert_eq!(gate.tail_remaining_ms(), DEFAULT_TAIL_MS);
        assert!(!gate.drain_allowed(), "the reverb tail must still block");
    }

    #[test]
    fn speaking_capture_advances_nothing() {
        let mut gate = HalfDuplexGate::with_default_tail();
        gate.begin_speaking();
        // A whole second of capture during playback changes nothing: the
        // speaking state, not elapsed time, is what blocks.
        gate.advance(1_000);
        assert!(!gate.drain_allowed());
        assert_eq!(gate.tail_remaining_ms(), 0);
    }

    #[test]
    fn begin_speaking_during_the_tail_cancels_it() {
        let mut gate = HalfDuplexGate::with_default_tail();
        gate.begin_speaking();
        gate.end_speaking();
        gate.advance(100);
        assert_eq!(gate.tail_remaining_ms(), 150);
        assert!(gate.begin_speaking());
        assert_eq!(gate.tail_remaining_ms(), 0);
        assert!(!gate.drain_allowed());
    }

    #[test]
    fn drain_allowed_once_the_tail_elapses() {
        let mut gate = HalfDuplexGate::with_default_tail();
        gate.begin_speaking();
        gate.end_speaking();
        gate.advance(249);
        assert!(!gate.drain_allowed(), "one millisecond of tail left");
        gate.advance(1);
        assert!(gate.drain_allowed(), "tail fully elapsed");
    }

    #[test]
    fn zero_tail_allows_draining_immediately_after_playback() {
        let mut gate = HalfDuplexGate::new(0);
        gate.begin_speaking();
        gate.end_speaking();
        assert!(gate.drain_allowed());
    }

    #[test]
    fn begin_capture_end_playback_drain_yields_nothing_then_capture_flows() {
        // The regression this gate exists for: audio captured while the
        // player spoke must never reach the pipeline, and audio captured
        // after the tail must.
        let mut gate = HalfDuplexGate::with_default_tail();

        assert!(gate.begin_speaking());
        // Playback happening; the runner drops whatever the mic buffered.
        gate.advance(1_000);
        assert!(
            !gate.drain_allowed(),
            "capture during playback is discarded"
        );

        gate.end_speaking();
        assert!(
            !gate.drain_allowed(),
            "capture during the reverb tail is discarded"
        );

        // The runner keeps dropping frames through the tail...
        gate.advance(DEFAULT_TAIL_MS);
        // ...and only then does capture reach the pipeline again.
        assert!(gate.drain_allowed());
        gate.advance(500);
        assert!(gate.drain_allowed(), "draining stays allowed afterwards");
    }
}
