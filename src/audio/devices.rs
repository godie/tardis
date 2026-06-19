//! Lists audio hosts and devices.
//!
//! In CPAL 0.18 device names must be printed via `{}`/`to_string()`
//! because `Device` implements `Display`; `Device::name()` does NOT exist.

use anyhow::Result;
use cpal::traits::HostTrait;

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
