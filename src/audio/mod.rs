//! Audio utilities for the Tardis CLI.
//!
//! The `activity` submodule classifies a pre-computed chunk volume into
//! `Silence` vs. `SpeechLike` against a configurable threshold.
//! The `chunk_recorder` submodule writes one WAV file per captured chunk.
//! The `chunker` submodule drives a real-time chunk-by-chunk capture test.
//! The `devices` submodule prints host/device information and exits.
//! The `mic` submodule handles microphone capture with simple volume logging.
//! The `recorder` submodule captures microphone audio to a WAV file.
//! The `volume` submodule holds generic pure-logic helpers covered by
//! unit tests.

pub mod activity;
pub mod chunk_recorder;
pub mod chunker;
pub mod devices;
pub mod mic;
pub mod recorder;
pub mod volume;
