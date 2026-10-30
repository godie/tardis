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

/// What the capture ring buffer holds when playback stops.
///
/// Playback blocks, so the buffer fills with our own voice. The tail alone
/// cannot absorb that backlog: a single drained chunk can be longer than
/// the tail, which leaves the rest of the playback audio queued up for the
/// transcriber. The whole backlog has to be dropped, partial samples
/// included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backlog {
    /// Complete chunks buffered during playback.
    pub chunks: usize,
    /// Samples after the last complete chunk. These are dropped too: keeping
    /// them would splice the tail of our own speech onto the next utterance.
    pub partial_samples: usize,
    /// Samples the backlog covers, `chunks * chunk_size + partial_samples`.
    pub total_samples: usize,
}

impl Backlog {
    /// Is there anything to clear?
    pub fn requires_clear(&self) -> bool {
        self.total_samples > 0
    }
}

/// Split `buffered_samples` into complete chunks and a partial remainder.
///
/// A zero `chunk_size` cannot split anything, so the whole buffer counts as
/// a partial remainder rather than dividing by zero.
pub fn playback_backlog(buffered_samples: usize, chunk_size: usize) -> Backlog {
    if chunk_size == 0 {
        return Backlog {
            chunks: 0,
            partial_samples: buffered_samples,
            total_samples: buffered_samples,
        };
    }
    Backlog {
        chunks: buffered_samples / chunk_size,
        partial_samples: buffered_samples % chunk_size,
        total_samples: buffered_samples,
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

    // ---- playback_backlog -----------------------------------------------

    #[test]
    fn empty_buffer_needs_no_clear() {
        let backlog = playback_backlog(0, 960);
        assert_eq!((backlog.chunks, backlog.partial_samples), (0, 0));
        assert!(!backlog.requires_clear());
    }

    #[test]
    fn whole_chunks_are_counted_without_a_remainder() {
        let backlog = playback_backlog(3 * 960, 960);
        assert_eq!((backlog.chunks, backlog.partial_samples), (3, 0));
        assert!(backlog.requires_clear());
    }

    #[test]
    fn partial_remainder_is_counted_and_dropped() {
        let backlog = playback_backlog(3 * 960 + 17, 960);
        assert_eq!((backlog.chunks, backlog.partial_samples), (3, 17));
        assert_eq!(backlog.total_samples, 3 * 960 + 17);
    }

    #[test]
    fn zero_chunk_size_counts_as_a_partial_remainder() {
        let backlog = playback_backlog(500, 0);
        assert_eq!((backlog.chunks, backlog.partial_samples), (0, 500));
        assert_eq!(backlog.total_samples, 500);
    }

    #[test]
    fn playback_backlog_never_reaches_the_transcriber() {
        // The regression: playback blocks, so the buffer fills with our own
        // voice. Dropping only what the 250 ms tail covers leaves the rest
        // queued, and a partial chunk survives to be spliced onto the next
        // utterance.
        let chunk_size = 960;
        let buffered = 4 * chunk_size + 17;
        let backlog = playback_backlog(buffered, chunk_size);

        assert_eq!(backlog.chunks, 4, "four full chunks of echo");
        assert_eq!(backlog.partial_samples, 17, "plus a partial remainder");
        assert!(backlog.requires_clear());

        // The runner clears the whole buffer, so nothing is left to hand to
        // the provider, complete chunks or partial samples.
        let remaining = buffered - backlog.total_samples;
        let after_clear = playback_backlog(remaining, chunk_size);
        assert_eq!(after_clear.chunks, 0, "no chunk reaches the transcriber");
        assert_eq!(after_clear.partial_samples, 0, "the remainder is gone");
        assert!(!after_clear.requires_clear());

        // Capture only flows again once the tail has elapsed.
        let mut gate = HalfDuplexGate::with_default_tail();
        gate.begin_speaking();
        gate.end_speaking();
        assert!(!gate.drain_allowed());
        gate.advance(DEFAULT_TAIL_MS);
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
