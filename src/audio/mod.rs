//! Audio utilities for the Tardis CLI.
//!
//! The `devices` submodule prints host/device information and exits.
//! The `mic` submodule handles microphone capture with simple volume logging.
//! The `volume` submodule holds pure-logic helpers covered by unit tests.

pub mod devices;
pub mod mic;
pub mod volume;
