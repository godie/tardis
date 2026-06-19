# tardis

Real-time audio capture + chunking CLI foundation, written in Rust. Captures audio from the default microphone, splits it into chunks, and either prints per-chunk statistics or feeds each chunk through a mock transcript classifier.

**Status:** [v0.1.0](https://github.com/godie/tardis/releases/tag/v0.1.0) — CLI foundation. Real transcription is **not yet integrated**; `mock-transcribe` simulates the pipeline shape, and only `mock_transcribe_chunk` will need to swap when a real backend lands.

## Features

| Mode                          | What it does                                                                          |
| ----------------------------- | ------------------------------------------------------------------------------------- |
| `cargo run -- devices`        | List the audio host + all available input/output devices.                             |
| `cargo run -- mic`            | Stream from the default mic and print avg volume every 500 ms (Ctrl+C to stop).       |
| `cargo run -- mic-5s`         | Capture for 5 seconds and exit.                                                       |
| `cargo run -- record-5s`      | Write 5 s of mic audio to `output/mic_test.wav` (16-bit PCM, channels + sample rate mirror the device). |
| `cargo run -- chunk-test`     | Capture 10 s, split into 1 s chunks, print per-chunk sample count + duration + avg volume (no WAV). |
| `cargo run -- mock-transcribe`| Capture 10 s, classify each 1 s chunk vs a volume threshold, print either a mock transcript line or a silence line. |

Run with no argument to default to `devices`.

## Installation

Requires the Rust toolchain (edition 2024) and a working microphone. macOS users must grant the terminal mic permission under *System Settings → Privacy & Security → Microphone* before the mic-aware modes will produce real audio.

```bash
git clone https://github.com/godie/tardis
cd tardis
cargo build --release
./target/release/tardis devices
```

## Usage

```bash
cargo run -- devices
cargo run -- mic
cargo run -- mic-5s
cargo run -- record-5s
cargo run -- chunk-test
cargo run -- mock-transcribe
```

Recorded WAVs land in `output/`, which is `.gitignore`d so capture artifacts don't pollute the repo.

## Project layout

```
src/
├── main.rs                          # CLI dispatch → one of six modes
├── audio/
│   ├── mod.rs
│   ├── devices.rs                   # cpal::default_host() + device listing
│   ├── mic.rs                       # default-mic capture loop (CPAL driver)
│   ├── volume.rs                    # pure volume helpers (14 unit tests)
│   ├── chunker.rs                   # pure chunking helpers + run_chunk_test (11 unit tests)
│   └── recorder.rs                  # WAV writer (hound)
└── transcription/
    ├── mod.rs
    ├── mock.rs                      # pure mock_transcribe_chunk (8 unit tests)
    └── pipeline.rs                  # CPAL driver for mock-transcribe
DEVELOPMENT_RULES.md                 # testing policy: pure logic → unit tests; never test CPAL / hardware
AGENTS.md                           # operational handbook for AI coding agents
LICENSE                             # MIT
```

## Testing

```bash
cargo test
```

33 unit tests pass across `audio::volume` (14), `audio::chunker` (11), and `transcription::mock` (8). Hardware behavior is exercised manually: every audio-side feature ships with a CLI mode above. The full policy lives in [`DEVELOPMENT_RULES.md`](./DEVELOPMENT_RULES.md).

## Dependencies

| Crate    | Version  | Role                                                       |
| -------- | -------- | ---------------------------------------------------------- |
| `cpal`   | `0.18`   | Cross-platform audio I/O.                                  |
| `anyhow` | `1.0.102`| Error plumbing.                                            |
| `hound`  | `3.5`    | 16-bit PCM WAV writer.                                     |

> **CPAL 0.18 note:** `cpal::StreamConfig::sample_rate` is a bare `u32` (not a tuple struct), `Device` no longer exposes `.name()` (use `format!("{device}")` / `Display`), and `Sample`/`FromSample` come from `dasp_sample`. A full list of traps is in `AGENTS.md`.

## License

[MIT](./LICENSE) — Copyright (c) 2026 godie.
