//! Default microphone capture with simple volume activity logging.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;
use cpal::traits::DeviceTrait;
use cpal::{FromSample, Sample, SizedSample};

use super::capture::DefaultInput;
use super::volume::{calculate_average_volume, is_above_threshold};
use crate::config::DEFAULT_VOLUME_THRESHOLD;

const LOG_COOLDOWN: Duration = Duration::from_millis(100);

/// Starts capturing audio from the default microphone and returns the
/// un-played `Stream`. Callers must `.play()?` the stream and hold it in
/// scope to keep capture alive; dropping it stops the stream.
pub fn start_default_mic_capture() -> Result<cpal::Stream> {
    let input = DefaultInput::resolve()?;
    println!("Mic device: {}", input.device_label());
    println!("Default input config: {}", input.supported_label());

    let stream = match input.sample_format() {
        cpal::SampleFormat::F32 => build_stream::<f32>(&input)?,
        cpal::SampleFormat::I16 => build_stream::<i16>(&input)?,
        cpal::SampleFormat::U16 => build_stream::<u16>(&input)?,
        other => return Err(anyhow::anyhow!("Unsupported sample format: {other:?}")),
    };

    Ok(stream)
}

fn build_stream<T>(input: &DefaultInput) -> Result<cpal::Stream>
where
    T: Sample + SizedSample,
    f32: FromSample<T>,
{
    // `Instant::now()` is not a const fn, so this can't live in a module-
    // level `const`. Set the timestamp to (now - cooldown) so the first
    // event above the threshold fires immediately.
    let last_log: Mutex<Instant> = Mutex::new(Instant::now() - LOG_COOLDOWN);

    let err_fn = |err| eprintln!("mic stream error: {}", err);

    let stream = input.device().build_input_stream(
        input.config(),
        move |data: &[T], _info: &cpal::InputCallbackInfo| {
            // Convert to f32 once so the testable volume helpers can
            // operate on a uniform slice. Buffer size is bounded by the
            // device's max-buffer-size config (≤ a few k samples), and
            // any allocation cost is acceptable for this CLI demo.
            // TODO(realtime): in production, switch to a lock-free
            // channel and a printer thread before this scales.
            let converted: Vec<f32> = data.iter().map(|&s| f32::from_sample(s)).collect();
            let avg = calculate_average_volume(&converted);
            if !is_above_threshold(avg, DEFAULT_VOLUME_THRESHOLD) {
                return;
            }
            if let Ok(mut last) = last_log.lock()
                && last.elapsed() >= LOG_COOLDOWN
            {
                println!("[mic] vol={:.3}", avg);
                *last = Instant::now();
            }
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}
