//! Audio utilities for the Tardis CLI.
//!
//! The `activity` submodule classifies a pre-computed chunk volume into
//! `Silence` vs. `SpeechLike` against a configurable threshold.
//! The `capture` submodule resolves the default input device and drains
//! capture chunks for every CPAL driver in the crate.
//! The `chunk_recorder` submodule writes one WAV file per captured chunk.
//! The `chunker` submodule drives a real-time chunk-by-chunk capture test.
//! The `devices` submodule prints host/device information and exits.
//! The `gate` submodule holds the pure half-duplex state machine that keeps
//! capture from hearing our own playback.
//! The `mic` submodule handles microphone capture with simple volume logging.
//! The `player` submodule holds the playback contract (`Player`) and its
//! deterministic `MockPlayer`, which records instead of playing.
//! The `recorder` submodule captures microphone audio to a WAV file.
//! The `volume` submodule holds generic pure-logic helpers covered by
//! unit tests.

pub mod activity;
pub mod capture;
pub mod chunk_recorder;
pub mod chunker;
pub mod devices;
pub mod gate;
pub mod mic;
pub mod player;
pub mod recorder;
pub mod volume;
