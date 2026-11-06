//! Playback stage: turn a rendered [`AudioChunk`] into sound.
//!
//! The trait lives here, next to the other CPAL glue
//! (`audio::capture`, `audio::mic`, `audio::recorder`), because a real
//! implementation owns an output stream and a queue. The mock
//! implementations record or count instead of playing, so the live
//! loop can be exercised end to end on a machine with no output
//! device, no TTS service, and no permissions.
//!
//! [`CpalPlayer`] is the real implementation: it owns a CPAL output
//! stream and a queue of interleaved samples already adapted to the
//! device's rate and channel count. Per DEVELOPMENT_RULES.md the trait,
//! the mocks and the pure adapters are unit-tested; the CPAL glue is
//! verified by `cargo run -- say-text` and the live `piper` path.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::synthesis::AudioChunk;
use crate::synthesis::local_http::resample_linear;

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
/// spoken. No CPAL stream, no device, no permissions. Keeping the samples
/// makes this unsuitable for a session that runs indefinitely; use
/// [`CountingPlayer`] there.
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

/// [`Player`] that counts instead of recording.
///
/// The live loop runs until the user stops it, so retaining a copy of every
/// rendered buffer would grow memory for the whole session. This keeps only
/// the counters the summary line reports and drops the audio, which is what
/// a player with no output device can honestly do.
#[derive(Debug, Default)]
pub struct CountingPlayer {
    chunks: usize,
    duration_ms: u64,
    stops: usize,
}

impl CountingPlayer {
    /// Construct a fresh `CountingPlayer` with nothing counted.
    pub fn new() -> Self {
        Self::default()
    }

    /// Chunks handed to `play_chunk` so far, including failed ones.
    pub fn played_count(&self) -> usize {
        self.chunks
    }

    /// Total rendered audio passed in, in milliseconds.
    pub fn played_duration_ms(&self) -> u64 {
        self.duration_ms
    }

    /// How many times `stop` was called. The contract makes `stop`
    /// idempotent, so this counts calls rather than transitions.
    pub fn stop_count(&self) -> usize {
        self.stops
    }
}

impl Player for CountingPlayer {
    fn play_chunk(&mut self, chunk: &AudioChunk) -> Result<()> {
        self.chunks += 1;
        self.duration_ms += crate::synthesis::chunk_duration_ms(chunk);
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        self.stops += 1;
        Ok(())
    }
}

// ===== CpalPlayer (real output, CPAL glue) ===============================

/// Real [`Player`] over the host's output device.
///
/// Owns one open CPAL output stream and a queue of interleaved `f32`
/// samples **already adapted to the device's rate and channel count**;
/// the audio callback only drains the queue and pads with silence, so
/// it allocates nothing and holds the lock for a single pop.
///
/// `play_chunk` blocks until the device is expected to have started
/// consuming the queue (not until the speakers finish the reverb tail,
/// which no queue can know about); the half-duplex gate in the runner
/// owns the tail.
pub struct CpalPlayer {
    /// Interleaved samples at the device's rate, drained by the
    /// callback. Shared with the audio callback; the runner only
    /// ever pushes and polls.
    queue: Arc<Mutex<Vec<f32>>>,
    /// The stream must be kept alive for the callback to run; dropping
    /// it detaches the device. It is never read directly, hence the
    /// underscore prefix.
    _stream: cpal::Stream,
    /// Device sample rate and channel count the queue is rendered at.
    device_rate: u32,
    device_channels: u16,
}

impl CpalPlayer {
    /// Open an output stream on the device named by `device_name`
    /// (case-insensitive substring match, `None` = the OS default) and
    /// start it.
    ///
    /// Fails with a readable error when the device does not exist or
    /// cannot be configured: the runner builds the player before the
    /// session announces "listening", so a bad device aborts startup
    /// instead of failing silently after every utterance.
    pub fn new(host: &cpal::Host, device_name: Option<&str>) -> Result<Self> {
        let (device, label) = select_output_device(host, device_name)?;
        let config = device
            .default_output_config()
            .context(format!("no default output config for device \"{label}\""))?;
        let device_rate = config.sample_rate();
        let device_channels = config.channels();
        let queue: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));

        let callback_queue = queue.clone();
        let error_fn = |error| eprintln!("output stream error: {error}");
        // CPAL hands the callback its preferred type; for a `f32` output
        // config this closure is `(&mut [f32], ...)`. Building for `f32`
        // keeps the whole path scale-free (i16/U16 configs are rejected
        // explicitly below, as the contract's simplification for the
        // voices we support).
        let sample_format = config.sample_format();
        anyhow::ensure!(
            sample_format == cpal::SampleFormat::F32,
            "output device \"{label}\" reports {sample_format:?}; only an F32 output config is supported"
        );
        let stream = device.build_output_stream(
            cpal::StreamConfig {
                channels: device_channels,
                // `SampleRate` is a plain `u32` in CPAL 0.18, not a
                // tuple struct (AGENTS.md gotcha list).
                sample_rate: device_rate,
                buffer_size: cpal::BufferSize::Default,
            },
            move |output: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                let mut guard = callback_queue
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                drain_output(output, &mut guard);
            },
            error_fn,
            None,
        )?;
        stream.play().context("start the output stream")?;

        Ok(Self {
            queue,
            _stream: stream,
            device_rate,
            device_channels,
        })
    }

    /// Device sample rate the queue is rendered at.
    pub fn device_rate(&self) -> u32 {
        self.device_rate
    }

    /// Device channel count the queue is rendered at.
    pub fn device_channels(&self) -> u16 {
        self.device_channels
    }
}

impl Player for CpalPlayer {
    fn play_chunk(&mut self, chunk: &AudioChunk) -> Result<()> {
        let samples = adapt_chunk_to_device(chunk, self.device_rate, self.device_channels)?;
        anyhow::ensure!(
            !samples.is_empty(),
            "an empty chunk never reaches the player"
        );

        let start = Instant::now();
        {
            let mut guard = self
                .queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.extend(samples);
        }

        // Block until the callback has consumed and rendered what we
        // pushed. `render_deadline` is the chunk's own play time plus
        // a grace period: draining the software queue is not enough,
        // the last callback buffer is only queued for the device at
        // that point, and returning early would drop the final
        // syllable in `say-text`. `stall_deadline` bounds the wait if
        // the device stops consuming.
        let duration_ms = crate::synthesis::chunk_duration_ms(chunk);
        let render_deadline = Duration::from_millis(duration_ms + 100);
        let stall_deadline = Duration::from_millis(duration_ms * 3 + 500);
        loop {
            let queue_has_samples = {
                let guard = self
                    .queue
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                !guard.is_empty()
            };
            let elapsed = start.elapsed();
            if !queue_has_samples && elapsed >= render_deadline {
                break;
            }
            if queue_has_samples && elapsed >= stall_deadline {
                // Clear what the device did not take: a stalled
                // callback must not play this chunk's tail later,
                // on top of the next utterance.
                let mut guard = self
                    .queue
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                guard.clear();
                anyhow::bail!(
                    "the output device did not consume the chunk within {duration_ms} ms x3"
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        // There is only the live queue to clear: the stream is shared
        // with the audio callback and stopping a stream mid-session is
        // not needed (stop happens at teardown, when the player is
        // dropped).
        self.queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        Ok(())
    }
}

/// Audio-callback core: drain the queue into `output`, pad with
/// silence.
///
/// Deliberately allocation-free (one copy, one in-place drain) because
/// this runs on the OS audio thread. Pure over its arguments;
/// unit-tested.
pub fn drain_output(output: &mut [f32], queue: &mut Vec<f32>) {
    let take = output.len().min(queue.len());
    output[..take].copy_from_slice(&queue[..take]);
    queue.drain(..take);
    // Pad the remainder with silence so the device never underruns
    // while the queue empties at the end of an utterance.
    for slot in &mut output[take..] {
        *slot = 0.0;
    }
}

/// Adapt one [`AudioChunk`] to the output device: fold to mono if the
/// chunk is interleaved, resample to the device rate, and duplicate
/// mono into every device channel.
///
/// Pure and unit-tested: this is where a wrong rate ratio would
/// produce chipmunk audio.
pub fn adapt_chunk_to_device(
    chunk: &AudioChunk,
    device_rate: u32,
    device_channels: u16,
) -> Result<Vec<f32>> {
    anyhow::ensure!(chunk.sample_rate > 0, "chunk sample rate must be positive");
    anyhow::ensure!(chunk.channels > 0, "chunk channel count must be positive");
    anyhow::ensure!(device_rate > 0, "device sample rate must be positive");
    anyhow::ensure!(device_channels > 0, "device channel count must be positive");

    let mono = fold_to_mono(&chunk.samples, chunk.channels)?;
    let resampled = if chunk.sample_rate == device_rate {
        mono
    } else {
        resample_linear(&mono, chunk.sample_rate, device_rate)
    };
    Ok(duplicate_to_channels(resampled, device_channels))
}

fn fold_to_mono(samples: &[f32], channels: u16) -> Result<Vec<f32>> {
    anyhow::ensure!(channels > 0, "channel count must be positive");
    if channels == 1 {
        return Ok(samples.to_vec());
    }
    // Interleave fold: average consecutive groups of `channels` frames.
    // A trailing partial group is truncated, matching the WAV behaviour
    // (a half-frame carries no audible information).
    let stride = channels as usize;
    let frames = samples.len() / stride;
    Ok((0..frames)
        .map(|frame| {
            let group = &samples[frame * stride..(frame + 1) * stride];
            group.iter().sum::<f32>() / stride as f32
        })
        .collect())
}

fn duplicate_to_channels(mono: Vec<f32>, channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return mono;
    }
    let mut out = Vec::with_capacity(mono.len() * channels as usize);
    for sample in mono {
        for _ in 0..channels {
            out.push(sample);
        }
    }
    out
}

/// Pick an output device: named match when `wanted` is set, else the
/// host default.
///
/// `output_device_matches` is the unit-tested part of the decision;
/// enumeration itself is CPAL glue.
pub fn select_output_device(
    host: &cpal::Host,
    wanted: Option<&str>,
) -> Result<(cpal::Device, String)> {
    if let Some(wanted) = wanted {
        let name = wanted.trim();
        anyhow::ensure!(!name.is_empty(), "the output device name must not be empty");
        for device in host.output_devices()? {
            // Report the device's real label, not the searched
            // substring: the caller prints it in errors, and "macbook"
            // as a device name is a dead end.
            if output_device_matches(&device.to_string(), name) {
                let label = device.to_string();
                return Ok((device, label));
            }
        }
        // List a few real names in the error so a typo is fixable on
        // the spot.
        let available: Vec<String> = host
            .output_devices()?
            .map(|device| device.to_string())
            .take(10)
            .collect();
        anyhow::bail!(
            "output device \"{name}\" was not found; available devices include: {}",
            available.join(" | ")
        );
    }
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("no default output device available"))?;
    let label = device.to_string();
    Ok((device, label))
}

/// Name match for an output device: case-insensitive substring,
/// trimmed on both inputs. Empty or whitespace-only `wanted` never
/// matches (the caller treats it as "not configured").
pub fn output_device_matches(device_label: &str, wanted: &str) -> bool {
    let wanted = wanted.trim().to_lowercase();
    !wanted.is_empty() && device_label.to_lowercase().contains(&wanted)
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- drain_output ----------------------------------------------------

    fn from_queue(samples: &[f32]) -> Vec<f32> {
        samples.to_vec()
    }

    #[test]
    fn drain_fills_output_from_the_front_of_the_queue() {
        let mut queue = from_queue(&[0.1, 0.2, 0.3]);
        let mut output = [0.0_f32; 2];
        drain_output(&mut output, &mut queue);
        assert_eq!(output, [0.1, 0.2]);
        assert_eq!(queue, vec![0.3]);
    }

    #[test]
    fn drain_pads_with_silence_when_the_queue_is_shorter() {
        let mut queue = from_queue(&[0.5]);
        let mut output = [9.9_f32; 4];
        drain_output(&mut output, &mut queue);
        assert_eq!(output[0], 0.5);
        assert!(output[1..].iter().all(|s| *s == 0.0));
        assert!(queue.is_empty());
    }

    #[test]
    fn drain_of_an_empty_queue_silences_the_output() {
        let mut queue: Vec<f32> = Vec::new();
        let mut output = [1.0_f32; 3];
        drain_output(&mut output, &mut queue);
        assert!(output.iter().all(|s| *s == 0.0));
    }

    // ---- fold_to_mono / duplicate_to_channels via adapt -------------------

    fn audio_chunk(samples: Vec<f32>, rate: u32, channels: u16) -> AudioChunk {
        AudioChunk {
            samples,
            sample_rate: rate,
            channels,
            is_final: true,
        }
    }

    #[test]
    fn adapt_of_a_mono_chunk_at_the_device_rate_is_identity() {
        let chunk = audio_chunk(vec![0.1, -0.2], 48_000, 1);
        let out = adapt_chunk_to_device(&chunk, 48_000, 1).unwrap();
        assert_eq!(out, vec![0.1, -0.2]);
    }

    #[test]
    fn adapt_folds_a_stereo_chunk_to_mono_then_to_stereo() {
        // Interleaved stereo: 0.2/0.0 and 0.4/0.0 average to 0.1 and 0.2.
        let chunk = audio_chunk(vec![0.2, 0.0, 0.4, 0.0], 48_000, 2);
        let out = adapt_chunk_to_device(&chunk, 48_000, 2).unwrap();
        assert_eq!(out, vec![0.1, 0.1, 0.2, 0.2]);
    }

    #[test]
    fn adapt_resamples_when_the_chunk_and_device_rates_differ() {
        // 2 frames at 2 Hz -> 4 Hz: 4 frames, positions 0, 0.5, 1, 1.
        let chunk = audio_chunk(vec![0.0, 1.0], 2_000, 1);
        let out = adapt_chunk_to_device(&chunk, 4_000, 1).unwrap();
        assert_eq!(out.len(), 4);
        assert!(
            (out[1] - 0.5).abs() < 1e-6,
            "midpoint must interpolate: {out:?}"
        );
        assert!((out[3] - 1.0).abs() < 1e-6, "clamp at the end: {out:?}");
    }

    #[test]
    fn adapt_rejects_a_zero_rate_or_channel_count() {
        let chunk = audio_chunk(vec![0.0], 0, 1);
        assert!(adapt_chunk_to_device(&chunk, 48_000, 1).is_err());
        let chunk = audio_chunk(vec![0.0], 16_000, 0);
        assert!(adapt_chunk_to_device(&chunk, 48_000, 1).is_err());
        let chunk = audio_chunk(vec![0.0], 16_000, 1);
        assert!(adapt_chunk_to_device(&chunk, 0, 1).is_err());
    }

    // ---- output_device_matches -------------------------------------------

    #[test]
    fn matching_is_case_insensitive_and_trimmed() {
        assert!(output_device_matches("MacBook Pro Speakers", "macbook"));
        assert!(output_device_matches("AirPods Pro", "airpods"));
        assert!(output_device_matches("USB Audio", "  usb audio  "));
    }

    #[test]
    fn no_match_when_the_label_does_not_contain_the_wanted_name() {
        assert!(!output_device_matches("MacBook Pro Speakers", "blackhole"));
    }

    #[test]
    fn an_empty_or_whitespace_wanted_name_never_matches() {
        for wanted in ["", "   "] {
            assert!(!output_device_matches("MacBook Pro Speakers", wanted));
        }
    }

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
    fn counting_player_keeps_counters_and_no_samples() {
        let mut player = CountingPlayer::new();
        player.play_chunk(&chunk(960)).unwrap();
        player.play_chunk(&chunk(1_920)).unwrap();
        assert_eq!(player.played_count(), 2);
        assert_eq!(player.played_duration_ms(), 180);
        player.stop().unwrap();
        player.stop().unwrap();
        assert_eq!(player.stop_count(), 2);
    }

    #[test]
    fn fresh_counting_player_is_empty() {
        let player = CountingPlayer::new();
        assert_eq!(player.played_count(), 0);
        assert_eq!(player.played_duration_ms(), 0);
        assert_eq!(player.stop_count(), 0);
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
