//! Lists audio hosts and devices.
//!
//! In CPAL 0.18 device names must be printed via `{}`/`to_string()`
//! because `Device` implements `Display`; `Device::name()` does NOT exist.

use anyhow::{Result, bail};
use cpal::traits::HostTrait;

use crate::audio::capture::{list_system_input_devices, system_audio_supported};

/// Prints the active audio host, available input/output devices, and
/// the default input/output devices. Returns `Ok(())` after printing.
pub fn print_device_info(host: &cpal::Host) -> Result<()> {
    println!("Audio Host: {:?}", host.id());

    println!("\nInput devices:");
    for (i, device) in host.input_devices()?.enumerate() {
        println!("  [{}] {}", i, device);
    }

    println!("\nOutput devices:");
    for (i, device) in host.output_devices()?.enumerate() {
        println!("  [{}] {}", i, device);
    }

    println!("\nDefault input device:");
    match host.default_input_device() {
        Some(device) => println!("  {}", device),
        None => println!("  (none)"),
    }

    println!("\nDefault output device:");
    match host.default_output_device() {
        Some(device) => println!("  {}", device),
        None => println!("  (none)"),
    }

    Ok(())
}

/// Prints only the output devices and the default, the list the real
/// player picks from (`devices --output`: the preflight a `piper`
/// session runs to choose a `player_device` substring).
pub fn print_output_devices(host: &cpal::Host) -> Result<()> {
    println!("Audio Host: {:?}", host.id());

    println!("\nOutput devices:");
    for (i, device) in host.output_devices()?.enumerate() {
        println!("  [{}] {}", i, device);
    }

    println!("\nDefault output device:");
    match host.default_output_device() {
        Some(device) => println!("  {}", device),
        None => println!("  (none)"),
    }

    Ok(())
}

/// Print the loopback inputs a `--source system` session can capture from
/// (`devices --source system`).
///
/// On a platform without a loopback backend this fails with the same
/// "not supported on this platform" error `SystemInput::start` returns, so
/// the preflight and the session agree. On macOS it lists every virtual
/// input whose name matches [`crate::audio::capture::is_system_audio_device_name`].
pub fn print_system_devices(host: &cpal::Host) -> Result<()> {
    if !system_audio_supported() {
        bail!("system audio capture is not supported on this platform; see docs/system-audio.md");
    }

    println!("Audio Host: {:?}", host.id());
    let devices = list_system_input_devices(host)?;
    if devices.is_empty() {
        bail!(
            "No system-audio loopback input device found. On macOS install BlackHole 2ch \
             and route the audio you want to transcribe into it; see docs/system-audio.md"
        );
    }

    println!("\nSystem audio input devices:");
    for (i, device) in devices.iter().enumerate() {
        println!("  [{}] {}", i, device.name);
    }

    println!("\nDefault system audio input device:");
    println!("  {}", devices[0].name);

    Ok(())
}
