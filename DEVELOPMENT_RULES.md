# Development Rules

This project follows three rules for every new feature. They exist so the
codebase stays verifiable on a machine with no real microphone, no Tauri UI,
and no internet — and so every hardware-facing feature has a cheap,
scriptable sanity check you can run by hand.

## The rules

### 1. Pure logic gets `#[cfg(test)] mod tests`

A function is pure when it depends only on its arguments (no globals, no
time, no IO, no thread-state) and returns the same output for the same
input. Any module that exposes pure helpers must include a
`#[cfg(test)] mod tests` block at the bottom of the file, exercising the
boundary cases, the equality edges, and any clamping/saturation behaviour
relevant to the float math. Tests run via `cargo test`.

If a behaviour is hard to test in isolation, factor it into a pure helper
and test that. Leave the impure glue (`build_input_stream`, file IO,
mutex-wrapped shared buffers) for manual checks.

### 2. Never unit test CPAL, real microphones, permissions, or physical devices

Hardware-facing code — anything that opens a CPAL stream, requires a real
input device, or depends on OS-level audio permissions — is not a `cargo
test` target. Can't be relied on in CI, can't be mocked meaningfully inside
a unit test. Don't try. Split the logic into a pure function so rule 1
covers the decision, then exercise the hardware glue manually (rule 3).

### 3. Every hardware/audio feature ships with a `cargo run -- <mode>`

Every new hardware-facing feature must be exposed as a small CLI mode the
user can run from the terminal. Add an entry to the match in `src/main.rs`,
update the mode docs at the top of that file, and update the table below.
Modes exit on their own (a fixed window + drain) and must clean up CPAL
streams on exit so they can be re-run.

## Existing coverage

The modules below already follow this split. When you add a new module,
mirror one of these rows.

| Module | Pure-logic test surface | Manual CLI command |
| --- | --- | --- |
| `audio::devices::print_device_info` | n/a (pure printing) | `cargo run -- devices` |
| `audio::volume` (avg / threshold / i16 / u16) | `tests` in `src/audio/volume.rs` | `cargo run -- mic`, `mic-5s` |
| `audio::chunker` (chunk-size + drain helpers) | `tests` in `src/audio/chunker.rs` | `cargo run -- chunk-test` |
| `audio::recorder::record_default_mic_to_wav_for_seconds` | n/a (hound + filesystem glue) | `cargo run -- record-5s` |
| `transcription::mock::mock_transcribe_chunk` | `tests` in `src/transcription/mock.rs` | `cargo run -- mock-transcribe` |
| `transcription::pipeline::run_mock_transcription_test` | n/a (CPAL driver) | `cargo run -- mock-transcribe` |

## How to add a new feature

1. Identify the decision logic. If it's pure (depends only on arguments),
   implement it as a `pub fn` in a small module and add a `#[cfg(test)] mod
   tests` covering the boundary cases. Examples: `mock_transcribe_chunk`,
   `drain_chunk`, `normalize_sample_i16`.
2. Identify the IO/hardware surface (CPAL stream, file write, thread
   spawn). Build it around the pure helper from step 1.
3. Wire it into `src/main.rs` as a new `cargo run -- <mode>` arm, update
   the mode docs at the top of the file, and add a matching row to the
   table above with the mode name(s) and the test location.
4. Run `cargo check && cargo test` to confirm the new pure-helper tests
   are wired up. Run `cargo run -- <mode>` by hand to confirm the
   hardware glue still works end-to-end.

## When to refactor

The capture-loop body (open host, pick device, build F32/I16/U16 stream,
tick-drain-drop) is currently duplicated across `audio/chunker.rs`,
`audio/recorder.rs`, and `transcription/pipeline.rs`. The third duplication
is the trigger to promote it to a shared helper with its own pure-logic
tests and a single canonical manual-command runner. Until then, keep the
duplication honest by updating all three sites whenever a new manual
command lands.
