//! Silence endpointing: whole utterances instead of clock chunks.
//!
//! The live pipeline used to cut on `chunk_duration_ms`, so the process
//! stage received one transcript per clock tick: a sentence could be split
//! in half, or two speakers could be concatenated. Endpointing at the
//! capture layer fixes the input contract for every stage above it.
//!
//! The decision is pure and frame-counted — no wall clock, no CPAL — so the
//! whole thing is unit-testable (DEVELOPMENT_RULES rule 1) while the
//! capture wiring stays on the manual path (rule 2). A *frame* is one
//! `chunk_duration_ms` batch of samples as the capture loop drains it,
//! classified speech-like or not by
//! [`crate::audio::activity::is_speech_like`]. There is no second VAD here:
//! the existing volume classifier is the signal endpointing needs.
//!
//! Rules, all table-tested below:
//!
//! 1. A speech-like frame opens an utterance.
//! 2. `hangover_ms` of consecutive non-speech frames closes it, provided it
//!    lasted at least `min_speech_ms`, so a pause mid-sentence does not end
//!    the utterance.
//! 3. A burst longer than `max_utterance_ms` is flushed at the cap and a new
//!    utterance starts immediately, so one long monologue cannot grow the
//!    buffer without bound.
//! 4. A burst shorter than `min_speech_ms` is discarded: no event, no WAV,
//!    no provider call.

use crate::config;

/// Endpointing thresholds, expressed in milliseconds because that is what
/// the UI and the config file carry. Converted to whole frames on use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointConfig {
    /// Capture granularity: one frame is this many milliseconds of audio.
    /// Comes from `AppRuntimeConfig::chunk_duration_ms`.
    pub chunk_duration_ms: u64,
    /// Consecutive non-speech time tolerated before an utterance closes.
    pub hangover_ms: u64,
    /// Shortest burst worth keeping; anything shorter is noise.
    pub min_speech_ms: u64,
    /// Burst cap: flush the utterance even if the speaker never paused.
    pub max_utterance_ms: u64,
}

impl Default for EndpointConfig {
    fn default() -> Self {
        Self {
            chunk_duration_ms: config::DEFAULT_CHUNK_DURATION_MS,
            hangover_ms: config::DEFAULT_HANGOVER_MS,
            min_speech_ms: config::DEFAULT_MIN_SPEECH_MS,
            max_utterance_ms: config::DEFAULT_MAX_UTTERANCE_MS,
        }
    }
}

/// What the caller should do with the frame it just fed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointDecision {
    /// The utterance is still open; keep buffering samples.
    Continue,
    /// The utterance closed after `frames` frames; flush the buffer once.
    Utterance { frames: usize },
    /// The burst was too short to be speech; drop the buffer.
    Discard,
}

/// Open-utterance accounting. Reset whenever an utterance closes or is
/// discarded, so a stale pause count cannot leak into the next one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EndpointState {
    open: bool,
    speech_frames: usize,
    total_frames: usize,
    silence_run: usize,
}

impl EndpointState {
    /// A state with no utterance open.
    pub fn new() -> Self {
        Self::default()
    }

    /// Is an utterance currently accumulating?
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Frames classified speech-like in the open utterance.
    pub fn speech_frames(&self) -> usize {
        self.speech_frames
    }

    /// Frames the open utterance spans, trailing silence included.
    pub fn total_frames(&self) -> usize {
        self.total_frames
    }

    /// Consecutive non-speech frames at the tail of the open utterance.
    pub fn silence_run(&self) -> usize {
        self.silence_run
    }

    /// Drop the open utterance. The runner calls this after flushing a
    /// completed utterance and whenever the half-duplex gate throws captured
    /// audio away (that audio was our own voice, so it cannot be part of the
    /// user's next utterance).
    pub fn reset(&mut self) {
        self.open = false;
        self.speech_frames = 0;
        self.total_frames = 0;
        self.silence_run = 0;
    }
}

/// How many whole frames `ms` covers, rounding up and never returning 0 for
/// a non-zero window: a 400 ms hangover on 1 s frames is one frame, because
/// half a frame is still a frame of audio.
pub fn frames_for_ms(ms: u64, chunk_duration_ms: u64) -> usize {
    if ms == 0 {
        return 0;
    }
    let chunk = chunk_duration_ms.max(1);
    (ms.saturating_add(chunk - 1) / chunk) as usize
}

/// Milliseconds covered by `frames`, the inverse of [`frames_for_ms`].
pub fn frames_to_ms(frames: usize, chunk_duration_ms: u64) -> u64 {
    frames as u64 * chunk_duration_ms
}

/// Feed one capture frame into `state` and report what to do with it.
///
/// The caller buffers the frame's samples *before* acting on the decision:
/// the frame that closes an utterance (the last of the hangover) belongs to
/// that utterance, and the frame that hits the cap belongs to the utterance
/// being flushed.
pub fn advance(
    state: &mut EndpointState,
    frame_is_speech_like: bool,
    cfg: &EndpointConfig,
) -> EndpointDecision {
    let max_frames = frames_for_ms(cfg.max_utterance_ms, cfg.chunk_duration_ms).max(1);
    let hangover_frames = frames_for_ms(cfg.hangover_ms, cfg.chunk_duration_ms);
    let min_frames = frames_for_ms(cfg.min_speech_ms, cfg.chunk_duration_ms).max(1);

    if frame_is_speech_like {
        if !state.open {
            state.reset();
            state.open = true;
        }
        state.speech_frames += 1;
        state.total_frames += 1;
        state.silence_run = 0;
    } else {
        if !state.open {
            return EndpointDecision::Continue;
        }
        state.total_frames += 1;
        state.silence_run += 1;
    }

    let capped = state.total_frames >= max_frames;
    let hangover_done = state.silence_run > 0 && state.silence_run >= hangover_frames;
    if !capped && !hangover_done {
        return EndpointDecision::Continue;
    }

    let frames = state.total_frames;
    let long_enough = state.speech_frames >= min_frames;
    state.reset();
    if capped && frame_is_speech_like {
        // The speaker never paused: the next frame starts a fresh utterance
        // straight away instead of waiting for another onset.
        state.open = true;
    }
    if long_enough {
        EndpointDecision::Utterance { frames }
    } else {
        EndpointDecision::Discard
    }
}

/// Flush decision for a session that ends while an utterance is still open.
///
/// Takes the state by reference so the caller can reset it afterwards; the
/// same `min_speech_ms` rule applies as for a normal close.
pub fn finish(state: &EndpointState, cfg: &EndpointConfig) -> EndpointDecision {
    if !state.open {
        return EndpointDecision::Continue;
    }
    let min_frames = frames_for_ms(cfg.min_speech_ms, cfg.chunk_duration_ms).max(1);
    if state.speech_frames >= min_frames {
        EndpointDecision::Utterance {
            frames: state.total_frames,
        }
    } else {
        EndpointDecision::Discard
    }
}

/// One printable line per fed frame, used by `cargo run -- utterance-test`.
///
/// Pure string building, so the manual mode's output shape is testable
/// without a microphone.
pub fn describe_decision(
    frame_index: usize,
    frame_is_speech_like: bool,
    decision: EndpointDecision,
    cfg: &EndpointConfig,
) -> String {
    let kind = if frame_is_speech_like {
        "speech "
    } else {
        "silence"
    };
    match decision {
        EndpointDecision::Continue => format!("frame {frame_index:>2}  {kind}  -> continue"),
        EndpointDecision::Utterance { frames } => format!(
            "frame {frame_index:>2}  {kind}  -> utterance ({frames} frame(s), {} ms)",
            frames_to_ms(frames, cfg.chunk_duration_ms)
        ),
        EndpointDecision::Discard => format!(
            "frame {frame_index:>2}  {kind}  -> discard (burst below {} ms)",
            cfg.min_speech_ms
        ),
    }
}
/// Manual check for the pure state machine: no microphone, no Docker.
///
/// Feeds a fixed synthetic frame pattern through [`advance`] and prints each
/// decision plus a one-line summary. The pattern covers silence, a normal
/// utterance with a pause inside it, a short blip that must be discarded and
/// a monologue long enough to hit the burst cap.
///
/// The demo uses slower frames than the app defaults on purpose: with the
/// default 1 s frames `min_speech_ms` rounds up to a single frame, so a
/// discard cannot be demonstrated at all. The production defaults stay in
/// [`crate::config`].
pub fn run_utterance_test() {
    let cfg = EndpointConfig {
        chunk_duration_ms: 250,
        hangover_ms: 500,        // 2 frames
        min_speech_ms: 750,      // 3 frames
        max_utterance_ms: 2_000, // 8 frames
    };
    println!("=== utterance-test (endpointing state machine, no mic) ===\n");
    println!(
        "frame:      {} ms\nhangover:   {} ms\nmin speech: {} ms\nmax utter.: {} ms\n",
        cfg.chunk_duration_ms, cfg.hangover_ms, cfg.min_speech_ms, cfg.max_utterance_ms
    );

    // (is_speech_like, frames) runs.
    let pattern: [(bool, usize); 7] = [
        (false, 2), // idle: nothing opens
        (true, 4),  // "hello" (1 s, above the 750 ms minimum)
        (false, 2), // the hangover elapses -> one utterance
        (true, 2),  // 500 ms blip: below the minimum
        (false, 2), // -> discarded, not transcribed
        (true, 12), // monologue: the 2 s cap flushes at 8 frames
        (false, 2), // the remaining 4 open a fresh utterance and close it
    ];

    let mut state = EndpointState::new();
    let mut frame_index = 0usize;
    let mut utterances = 0usize;
    let mut discards = 0usize;

    for (is_speech, count) in pattern {
        for _ in 0..count {
            frame_index += 1;
            let decision = advance(&mut state, is_speech, &cfg);
            match decision {
                EndpointDecision::Utterance { .. } => utterances += 1,
                EndpointDecision::Discard => discards += 1,
                EndpointDecision::Continue => {}
            }
            println!(
                "{}",
                describe_decision(frame_index, is_speech, decision, &cfg)
            );
        }
    }

    match finish(&state, &cfg) {
        EndpointDecision::Utterance { frames } => {
            utterances += 1;
            println!(
                "end     open utterance  -> utterance ({frames} frame(s), {} ms)",
                frames_to_ms(frames, cfg.chunk_duration_ms)
            );
        }
        EndpointDecision::Discard => {
            discards += 1;
            println!("end     open utterance  -> discard (below min_speech_ms)");
        }
        EndpointDecision::Continue => println!("end     nothing open    -> nothing to flush"),
    }

    println!(
        "\n=== summary ===\n{frame_index} frames, {utterances} utterance(s) flushed, {discards} discarded"
    );
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 1 s frames keep the arithmetic readable: one frame is 1000 ms.
    fn cfg() -> EndpointConfig {
        EndpointConfig {
            chunk_duration_ms: 1000,
            hangover_ms: 1000,
            min_speech_ms: 1000,
            max_utterance_ms: 10_000,
        }
    }

    fn feed(
        state: &mut EndpointState,
        cfg: &EndpointConfig,
        frames: &[bool],
    ) -> Vec<EndpointDecision> {
        frames
            .iter()
            .map(|&is_speech| advance(state, is_speech, cfg))
            .collect()
    }

    // ---- frames_for_ms / frames_to_ms ----------------------------------

    #[test]
    fn frames_for_ms_zero_is_zero() {
        assert_eq!(frames_for_ms(0, 1000), 0);
    }

    #[test]
    fn frames_for_ms_rounds_up_a_partial_frame() {
        assert_eq!(frames_for_ms(400, 1000), 1, "400 ms needs the whole frame");
        assert_eq!(frames_for_ms(1000, 1000), 1);
        assert_eq!(frames_for_ms(1001, 1000), 2);
    }

    #[test]
    fn frames_for_ms_never_divides_by_zero() {
        assert_eq!(frames_for_ms(500, 0), 500);
    }

    #[test]
    fn frames_to_ms_inverts_frames_for_ms() {
        assert_eq!(frames_to_ms(3, 1000), 3000);
        assert_eq!(frames_to_ms(frames_for_ms(400, 250), 250), 500);
    }

    // ---- opening and closing ------------------------------------------

    #[test]
    fn silence_before_speech_never_opens_an_utterance() {
        let cfg = cfg();
        let mut state = EndpointState::new();
        let decisions = feed(&mut state, &cfg, &[false, false, false]);
        assert_eq!(decisions, vec![EndpointDecision::Continue; 3]);
        assert!(!state.is_open());
    }

    #[test]
    fn speech_opens_the_utterance_and_keeps_it_open() {
        let cfg = cfg();
        let mut state = EndpointState::new();
        assert_eq!(
            advance(&mut state, true, &cfg),
            EndpointDecision::Continue,
            "an utterance is not emitted before its hangover elapses"
        );
        assert!(state.is_open());
        assert_eq!(state.speech_frames(), 1);
        assert_eq!(state.total_frames(), 1);
    }

    #[test]
    fn hangover_closes_the_utterance_including_the_trailing_silence() {
        let cfg = cfg();
        let mut state = EndpointState::new();
        let decisions = feed(&mut state, &cfg, &[true, true, false]);
        assert_eq!(
            decisions,
            vec![
                EndpointDecision::Continue,
                EndpointDecision::Continue,
                EndpointDecision::Utterance { frames: 3 },
            ],
            "two speech frames plus the closing silence frame"
        );
        assert!(!state.is_open());
    }

    #[test]
    fn pause_shorter_than_hangover_keeps_the_utterance_open() {
        let cfg = EndpointConfig {
            hangover_ms: 3000,
            ..cfg()
        };
        let mut state = EndpointState::new();
        let decisions = feed(&mut state, &cfg, &[true, false, false]);
        assert_eq!(
            decisions,
            vec![EndpointDecision::Continue; 3],
            "3 s of hangover needs three silence frames, not two"
        );
        assert!(state.is_open());
        assert_eq!(state.silence_run(), 2);
    }

    #[test]
    fn pause_longer_than_hangover_closes_once() {
        let cfg = EndpointConfig {
            hangover_ms: 2000,
            ..cfg()
        };
        let mut state = EndpointState::new();
        let decisions = feed(&mut state, &cfg, &[true, false, false, false]);
        assert_eq!(
            decisions[..2],
            [EndpointDecision::Continue; 2],
            "one silence frame is not yet the 2 s hangover"
        );
        assert_eq!(
            decisions[2],
            EndpointDecision::Utterance { frames: 3 },
            "the second silence frame closes it, once"
        );
        assert_eq!(
            decisions[3],
            EndpointDecision::Continue,
            "with nothing open, later silence does nothing"
        );
        assert!(!state.is_open());
    }

    #[test]
    fn short_blip_is_discarded_not_transcribed() {
        let cfg = EndpointConfig {
            min_speech_ms: 2000,
            ..cfg()
        };
        let mut state = EndpointState::new();
        let decisions = feed(&mut state, &cfg, &[true, false]);
        assert_eq!(
            decisions,
            vec![EndpointDecision::Continue, EndpointDecision::Discard],
            "one speech frame is below the 2 s minimum"
        );
        assert!(!state.is_open());
    }

    #[test]
    fn burst_exactly_at_min_speech_is_accepted() {
        let cfg = EndpointConfig {
            min_speech_ms: 2000,
            ..cfg()
        };
        let mut state = EndpointState::new();
        let decisions = feed(&mut state, &cfg, &[true, true, false]);
        assert_eq!(
            decisions[2],
            EndpointDecision::Utterance { frames: 3 },
            "the boundary is inclusive"
        );
    }

    #[test]
    fn burst_at_max_utterance_flushes_at_the_cap_and_reopens() {
        let cfg = EndpointConfig {
            max_utterance_ms: 3000,
            ..cfg()
        };
        let mut state = EndpointState::new();
        let decisions = feed(&mut state, &cfg, &[true, true, true]);
        assert_eq!(
            decisions[..2],
            [EndpointDecision::Continue; 2],
            "the first two frames are still accumulating"
        );
        assert_eq!(
            decisions[2],
            EndpointDecision::Utterance { frames: 3 },
            "the cap flushes at three frames"
        );
        assert!(state.is_open(), "a still-speaking frame reopens a new one");
        assert_eq!(state.speech_frames(), 0, "the new utterance is empty");

        assert_eq!(
            advance(&mut state, true, &cfg),
            EndpointDecision::Continue,
            "the frame after the flush starts the next utterance"
        );
        assert_eq!(state.speech_frames(), 1);
    }

    #[test]
    fn frames_after_the_cap_flush_start_a_new_utterance() {
        let cfg = EndpointConfig {
            max_utterance_ms: 3000,
            ..cfg()
        };
        let mut state = EndpointState::new();
        let decisions = feed(&mut state, &cfg, &[true, true, true, true, false]);
        assert_eq!(
            decisions[4],
            EndpointDecision::Utterance { frames: 2 },
            "one frame in the new utterance plus the closing silence"
        );
        assert!(!state.is_open());
    }

    #[test]
    fn zero_hangover_closes_on_the_first_silence_frame() {
        let cfg = EndpointConfig {
            hangover_ms: 0,
            ..cfg()
        };
        let mut state = EndpointState::new();
        let decisions = feed(&mut state, &cfg, &[true, false]);
        assert_eq!(decisions[1], EndpointDecision::Utterance { frames: 2 });
    }

    #[test]
    fn silence_only_stream_never_yields_an_utterance() {
        let cfg = cfg();
        let mut state = EndpointState::new();
        let decisions = feed(&mut state, &cfg, &[false; 30]);
        assert!(
            !decisions
                .iter()
                .any(|d| matches!(d, EndpointDecision::Utterance { .. })),
            "silence alone must never reach the provider"
        );
    }

    #[test]
    fn reset_drops_every_counter() {
        let cfg = cfg();
        let mut state = EndpointState::new();
        feed(&mut state, &cfg, &[true, false]);
        state.reset();
        assert_eq!(state, EndpointState::new());
    }

    // ---- finish -------------------------------------------------------

    #[test]
    fn finish_flushes_an_open_utterance() {
        let cfg = cfg();
        let mut state = EndpointState::new();
        feed(&mut state, &cfg, &[true, true]);
        assert_eq!(
            finish(&state, &cfg),
            EndpointDecision::Utterance { frames: 2 }
        );
    }

    #[test]
    fn finish_discards_a_short_open_utterance() {
        let cfg = EndpointConfig {
            min_speech_ms: 3000,
            ..cfg()
        };
        let mut state = EndpointState::new();
        feed(&mut state, &cfg, &[true]);
        assert_eq!(finish(&state, &cfg), EndpointDecision::Discard);
    }

    #[test]
    fn finish_with_nothing_open_continues() {
        assert_eq!(
            finish(&EndpointState::new(), &cfg()),
            EndpointDecision::Continue
        );
    }

    // ---- describe_decision --------------------------------------------

    #[test]
    fn describe_decision_names_the_outcome() {
        let cfg = cfg();
        assert!(describe_decision(3, false, EndpointDecision::Continue, &cfg).contains("continue"));
        let line = describe_decision(4, true, EndpointDecision::Utterance { frames: 3 }, &cfg);
        assert!(
            line.contains("utterance") && line.contains("3000 ms"),
            "{line}"
        );
        let line = describe_decision(5, false, EndpointDecision::Discard, &cfg);
        assert!(
            line.contains("discard") && line.contains("1000 ms"),
            "{line}"
        );
    }
}
