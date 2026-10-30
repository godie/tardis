//! Shared CPAL input plumbing.
//!
//! Every chunked capture driver (`chunker`, `chunk_recorder`,
//! `transcription::pipeline`, `processor::pipeline`,
//! `transcription::live_local`) used to resolve the default input device,
//! build the same f32 callback stream, and drain complete chunks out of the
//! same shared buffer. This module owns that machinery; the drivers keep only
//! their own loop shape, banner text, and per-chunk work.
//!
//! Device enumeration and stream building touch CPAL and real hardware, so
//! they are not unit-tested (DEVELOPMENT_RULES rule 2). The pure pieces this
//! module stands on (`chunker::chunk_size_or_error`,
//! `chunker::has_complete_chunk`, `chunker::drain_chunk`) are covered by their
//! own tests, and so is the mono mix ([`mono_mix_weights`]): it is plain
//! arithmetic on a slice of peaks.

use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, StreamConfig};

use crate::audio::chunker::{chunk_size_or_error, drain_chunk, has_complete_chunk};

/// Default input device plus its stream config, resolved once.
///
/// The two labels are pre-formatted so each driver can print its own banner
/// without deriving the device/config text again.
pub struct DefaultInput {
    device: cpal::Device,
    config: StreamConfig,
    sample_format: cpal::SampleFormat,
    device_label: String,
    supported_label: String,
}

impl DefaultInput {
    /// Resolve the default input device and its default stream config.
    pub fn resolve() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| anyhow!("No default input device available"))?;
        let supported = device.default_input_config()?;
        Ok(Self {
            device_label: format!("{device}"),
            supported_label: format!("{supported:?}"),
            sample_format: supported.sample_format(),
            config: supported.into(),
            device,
        })
    }

    pub fn device(&self) -> &cpal::Device {
        &self.device
    }

    pub fn config(&self) -> StreamConfig {
        self.config
    }

    pub fn sample_format(&self) -> cpal::SampleFormat {
        self.sample_format
    }

    /// `Display` form of the device, as printed in every driver banner.
    pub fn device_label(&self) -> &str {
        &self.device_label
    }

    /// `Debug` form of the device's default config, as printed in banners.
    pub fn supported_label(&self) -> &str {
        &self.supported_label
    }

    pub fn sample_rate(&self) -> u32 {
        self.config.sample_rate
    }

    /// Channels the device delivers, interleaved.
    ///
    /// This is not what the drivers see: virtual loopback devices (BlackHole
    /// 16ch / 64ch) report 16 or 64 channels while the signal only occupies
    /// one or two of them, so the stream folds every frame down to one
    /// channel before it reaches the buffer. See [`mono_mix_weights`].
    pub fn device_channels(&self) -> u16 {
        self.config.channels
    }
}

/// The buffer, the gate and the provider all work in mono.
const MONO_CHANNELS: u16 = 1;

/// Most channels a single block may carry into the mono mix.
///
/// CoreAudio's widest device here is BlackHole 64ch; anything above this is
/// ignored rather than allowed to walk off a fixed-size array.
const MAX_MIX_CHANNELS: usize = 64;

/// A channel joins the mono mix only when its peak in the block reaches this
/// fraction of the loudest channel's peak.
///
/// `0.5` keeps a stereo pair together (both sides of real audio land within
/// a factor of two) while dropping silent padding channels entirely, which is
/// the difference between speech at full level and speech at 1/16 of it.
const MONO_ENERGY_RATIO: f32 = 0.5;

/// Per-channel mix weights for one callback block.
///
/// The signal is **not** assumed to live on channels 0 and 1: a virtual
/// loopback device pads with silence, but a multi-input interface can put the
/// voice on channel 3 or 7 and leave the first pair empty. So the mix is
/// built from whichever channels actually carry energy in this block — those
/// within [`MONO_ENERGY_RATIO`] of the loudest one — and each gets an equal
/// share. Silent channels get weight 0, which is what keeps the level intact:
/// averaging all 16 channels of a BlackHole device divides the amplitude by 8,
/// and loud speech then reads as silence to the volume gate.
///
/// Returns a fixed-size array so the audio callback never allocates.
pub(crate) fn mono_mix_weights(
    peaks: &[f32; MAX_MIX_CHANNELS],
    channels: usize,
) -> [f32; MAX_MIX_CHANNELS] {
    let mut weights = [0.0_f32; MAX_MIX_CHANNELS];
    let considered = channels.min(MAX_MIX_CHANNELS);
    let loudest = peaks[..considered].iter().copied().fold(0.0_f32, f32::max);
    if loudest <= 0.0 {
        return weights;
    }

    let gate = loudest * MONO_ENERGY_RATIO;
    let mut active = 0_usize;
    for (weight, peak) in weights.iter_mut().zip(&peaks[..considered]) {
        if *peak >= gate {
            *weight = 1.0;
            active += 1;
        }
    }
    if active == 0 {
        // Cannot happen with `gate <= loudest`, but a zero-weight block is
        // silence rather than an infinite share.
        return weights;
    }

    let share = 1.0 / active as f32;
    for weight in &mut weights {
        *weight *= share;
    }
    weights
}

/// Per-channel peaks for one callback block. Allocation-free.
fn block_peaks<T>(data: &[T], channels: usize, peaks: &mut [f32; MAX_MIX_CHANNELS])
where
    T: Copy,
    f32: FromSample<T>,
{
    for frame in data.chunks(channels) {
        for (i, sample) in frame.iter().enumerate().take(MAX_MIX_CHANNELS) {
            let value = f32::from_sample(*sample).abs();
            if value > peaks[i] {
                peaks[i] = value;
            }
        }
    }
}

/// Mix one block down to mono with `weights` and append it to `buffer`.
///
/// Pushes one sample per frame, so a stereo device and a 64-channel device
/// both advance the buffer at the sample rate. Allocation-free apart from the
/// buffer's own amortized growth, which is bounded by the drain in the
/// capture loop.
fn push_mono<T>(
    buffer: &mut Vec<f32>,
    data: &[T],
    channels: usize,
    weights: &[f32; MAX_MIX_CHANNELS],
) where
    T: Copy,
    f32: FromSample<T>,
{
    for frame in data.chunks(channels) {
        let mut acc = 0.0_f32;
        for (i, sample) in frame.iter().enumerate().take(MAX_MIX_CHANNELS) {
            acc += weights[i] * f32::from_sample(*sample);
        }
        buffer.push(acc);
    }
}

/// An unplayed capture stream feeding mono f32 samples into a shared buffer,
/// plus the input config those samples were captured with.
pub struct MicInput {
    input: DefaultInput,
    buffer: Arc<Mutex<Vec<f32>>>,
    stream: cpal::Stream,
}

impl MicInput {
    /// Resolve the default input device and build its capture stream.
    /// `error_prefix` labels stream errors, e.g. `"chunk recorder stream
    /// error: …"`.
    pub fn start(error_prefix: &'static str) -> Result<Self> {
        let input = DefaultInput::resolve()?;
        let buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
        let stream = match input.sample_format() {
            cpal::SampleFormat::F32 => {
                build_stream::<f32>(&input, Arc::clone(&buffer), error_prefix)?
            }
            cpal::SampleFormat::I16 => {
                build_stream::<i16>(&input, Arc::clone(&buffer), error_prefix)?
            }
            cpal::SampleFormat::U16 => {
                build_stream::<u16>(&input, Arc::clone(&buffer), error_prefix)?
            }
            other => return Err(anyhow!("Unsupported sample format: {other:?}")),
        };
        Ok(Self {
            input,
            buffer,
            stream,
        })
    }

    pub fn input(&self) -> &DefaultInput {
        &self.input
    }

    pub fn sample_rate(&self) -> u32 {
        self.input.sample_rate()
    }

    /// Always 1: [`build_stream`] downmixes every block on the way in, so the
    /// buffer, the volume gate, the endpointing frames and the WAV handed to
    /// the provider are all mono.
    pub fn channels(&self) -> u16 {
        MONO_CHANNELS
    }

    /// Channels the hardware reports (16 for BlackHole 16ch), for banners.
    pub fn device_channels(&self) -> u16 {
        self.input.device_channels()
    }

    /// Samples per chunk for `chunk_duration_ms`, or an error when the device
    /// config makes that zero.
    pub fn chunk_size(&self, chunk_duration_ms: u64) -> Result<usize> {
        chunk_size_or_error(self.sample_rate(), self.channels(), chunk_duration_ms)
    }

    pub fn play(&self) -> Result<()> {
        self.stream.play()?;
        Ok(())
    }

    /// Drain one complete chunk if the buffer holds at least `chunk_size`
    /// samples; leftover samples stay buffered for the next call.
    pub fn next_chunk(&self, chunk_size: usize) -> Option<Vec<f32>> {
        let mut buffer = match self.buffer.lock() {
            Ok(buffer) => buffer,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !has_complete_chunk(buffer.len(), chunk_size) {
            return None;
        }
        drain_chunk(&mut buffer, chunk_size)
    }

    /// Drop everything currently buffered, partial samples included, and
    /// return how many samples were discarded.
    ///
    /// The half-duplex gate calls this the moment playback ends: audio
    /// captured while the player spoke is our own voice, and a leftover
    /// partial chunk would otherwise be glued onto the next utterance.
    /// Draining chunk by chunk cannot do this, because the drain stops as
    /// soon as the reverb tail is consumed and the rest stays queued.
    pub fn clear(&self) -> usize {
        let mut buffer = match self.buffer.lock() {
            Ok(buffer) => buffer,
            Err(poisoned) => poisoned.into_inner(),
        };
        let dropped = buffer.len();
        buffer.clear();
        dropped
    }
}

fn build_stream<T>(
    input: &DefaultInput,
    buffer: Arc<Mutex<Vec<f32>>>,
    error_prefix: &'static str,
) -> Result<cpal::Stream>
where
    T: Sample + cpal::SizedSample,
    f32: FromSample<T>,
{
    let device_channels = input.device_channels() as usize;
    let stream = input.device().build_input_stream(
        input.config(),
        move |data: &[T], _info: &cpal::InputCallbackInfo| {
            // Mono in, mono out: one buffer sample per frame regardless of
            // how many channels the device reports. The scratch arrays are
            // stack locals: this callback runs on the OS audio thread and
            // must not allocate (AGENTS.md).
            let mut peaks = [0.0_f32; MAX_MIX_CHANNELS];
            block_peaks(data, device_channels, &mut peaks);
            let weights = mono_mix_weights(&peaks, device_channels);

            let mut buffer = match buffer.lock() {
                Ok(buffer) => buffer,
                Err(poisoned) => poisoned.into_inner(),
            };
            push_mono(&mut buffer, data, device_channels, &weights);
        },
        move |err| eprintln!("{error_prefix} stream error: {err}"),
        None,
    )?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mix one frame of `channels` samples with the weights those peaks
    /// produce, the way `push_mono` does it in the callback.
    fn mix(frame: &[f32]) -> f32 {
        let peaks = peaks_of(frame, frame.len().max(1));
        let weights = mono_mix_weights(&peaks, frame.len().max(1));
        frame
            .iter()
            .zip(weights.iter())
            .map(|(sample, weight)| weight * sample)
            .sum()
    }

    fn peaks_of(frame: &[f32], channels: usize) -> [f32; MAX_MIX_CHANNELS] {
        let mut peaks = [0.0_f32; MAX_MIX_CHANNELS];
        block_peaks(frame, channels, &mut peaks);
        peaks
    }

    #[test]
    fn mono_device_passes_through() {
        assert_eq!(mix(&[0.5]), 0.5);
    }

    #[test]
    fn stereo_pair_is_averaged() {
        assert!((mix(&[0.5, 0.3]) - 0.4).abs() < 1e-6);
    }

    #[test]
    fn sixteen_channels_with_signal_on_the_first_pair_keep_full_level() {
        // The BlackHole 16ch case: signal on the first pair, 14 silent
        // channels. Averaging all 16 would mix to 0.1 here, below the 0.01
        // gate for ordinary loudness, and speech would look like silence.
        let mut frame = [0.0_f32; 16];
        frame[0] = 0.8;
        frame[1] = 0.8;
        assert!((mix(&frame) - 0.8).abs() < 1e-6);
    }

    #[test]
    fn a_signal_on_a_later_channel_is_not_lost() {
        // An audio interface can put the voice on channel 3 and leave the
        // assumed first pair empty. The mix follows the energy, not the index.
        let mut frame = [0.0_f32; 16];
        frame[3] = 0.6;
        assert!((mix(&frame) - 0.6).abs() < 1e-6);
    }

    #[test]
    fn the_loudest_channel_dominates_a_much_quieter_one() {
        // 0.1 is below half of 0.8, so it does not halve the level. Real
        // stereo pairs stay within 2x of each other and are averaged.
        assert!((mix(&[0.8, 0.1]) - 0.8).abs() < 1e-6);
    }

    #[test]
    fn a_silent_block_mixes_to_zero() {
        assert_eq!(mix(&[0.0; 16]), 0.0);
    }

    #[test]
    fn channels_beyond_the_fixed_array_are_ignored_not_panicked() {
        // BlackHole 64ch is the widest device here; anything wider must be
        // clipped to the array rather than index out of bounds.
        let peaks = peaks_of(&[0.3_f32; 80], 80);
        let weights = mono_mix_weights(&peaks, 200);
        assert_eq!(weights.len(), MAX_MIX_CHANNELS);
        assert!(weights.iter().all(|w| w.is_finite()));
    }
}
