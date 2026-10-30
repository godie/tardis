//! Playback stage: turn a rendered [`AudioChunk`] into sound.
//!
//! The trait lives here, next to the other CPAL glue
//! (`audio::capture`, `audio::mic`, `audio::recorder`), because a real
//! implementation owns an output stream and a queue. The mock
//! implementation records what it was asked to play and plays
//! nothing, so the live loop can be exercised end to end on a machine
//! with no output device, no TTS service, and no permissions.
//!
//! Per DEVELOPMENT_RULES.md: the trait and the mock are pure logic and
//! are unit-tested; the CPAL-backed player lands later and is verified
//! by hand with a `cargo run -- <mode>` command.

use std::sync::Mutex;

use anyhow::Result;

use crate::synthesis::AudioChunk;

/// Synchronous playback contract.
///
/// `&mut self` (unlike the other stage traits, which take `&self`),
/// because a real player owns a stream handle and a queue, and starting a
/// chunk mutates that state. `stop` must be safe to call when nothing
/// is playing and more than once.
pub trait Player {
    /// Play one rendered chunk. Blocking for the duration of the
    /// chunk is acceptable for the first real implementation; the
    /// streaming interface in the plan replaces this wholesale.
    fn play_chunk(&mut self, chunk: &AudioChunk) -> Result<()>;

    /// Stop playback and drop anything queued. Idempotent.
    fn stop(&mut self) -> Result<()>;
}

/// [`Player`] that records instead of playing.
///
/// Every successful `play_chunk` appends a clone of the chunk, so the
/// runner and its tests can assert exactly what would have been
/// spoken. No CPAL stream, no device, no permissions.
#[derive(Debug, Default)]
pub struct MockPlayer {
    played: Mutex<Vec<AudioChunk>>,
}

impl MockPlayer {
    /// Construct a fresh `MockPlayer` with nothing recorded.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of chunks recorded so far.
    pub fn played_count(&self) -> usize {
        self.lock().len()
    }

    /// Snapshot of the recorded chunks, in play order.
    pub fn played_chunks(&self) -> Vec<AudioChunk> {
        self.lock().clone()
    }

    /// Total recorded audio, in milliseconds. Uses the same helper as
    /// the UI so the numbers agree.
    pub fn played_duration_ms(&self) -> u64 {
        self.lock()
            .iter()
            .map(crate::synthesis::chunk_duration_ms)
            .sum()
    }

    /// A panic inside `play_chunk` cannot happen (the body only pushes
    /// into a `Vec`), but recovering from a poisoned mutex keeps one
    /// bad thread from wedging the recorder for the rest of the run.
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<AudioChunk>> {
        self.played
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Player for MockPlayer {
    fn play_chunk(&mut self, chunk: &AudioChunk) -> Result<()> {
        self.lock().push(chunk.clone());
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        // Nothing to tear down: the mock never started anything.
        Ok(())
    }
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(frames: usize) -> AudioChunk {
        AudioChunk {
            samples: vec![0.1; frames],
            sample_rate: 16_000,
            channels: 1,
            is_final: true,
        }
    }

    #[test]
    fn new_player_has_played_nothing() {
        let player = MockPlayer::new();
        assert_eq!(player.played_count(), 0);
        assert!(player.played_chunks().is_empty());
    }

    #[test]
    fn play_chunk_records_exactly_one_chunk() {
        let mut player = MockPlayer::new();
        player
            .play_chunk(&chunk(960))
            .expect("mock play must succeed");
        assert_eq!(player.played_count(), 1);
    }

    #[test]
    fn play_chunk_records_the_chunk_verbatim() {
        let mut player = MockPlayer::new();
        let recorded = chunk(960);
        player
            .play_chunk(&recorded)
            .expect("mock play must succeed");
        assert_eq!(player.played_chunks(), vec![recorded]);
    }

    #[test]
    fn two_plays_record_two_chunks_in_order() {
        let mut player = MockPlayer::new();
        player.play_chunk(&chunk(960)).unwrap();
        player.play_chunk(&chunk(1_920)).unwrap();
        let played = player.played_chunks();
        assert_eq!(played.len(), 2);
        assert_eq!(played[0].samples.len(), 960);
        assert_eq!(played[1].samples.len(), 1_920);
    }

    #[test]
    fn played_duration_sums_each_chunk() {
        let mut player = MockPlayer::new();
        // 960 frames @16 kHz = 60 ms; 1 920 frames = 120 ms.
        player.play_chunk(&chunk(960)).unwrap();
        player.play_chunk(&chunk(1_920)).unwrap();
        assert_eq!(player.played_duration_ms(), 180);
    }

    #[test]
    fn stop_is_idempotent_and_keeps_the_recording() {
        let mut player = MockPlayer::new();
        player.play_chunk(&chunk(960)).unwrap();
        player.stop().expect("first stop must succeed");
        player.stop().expect("second stop must succeed");
        assert_eq!(player.played_count(), 1);
    }
}
