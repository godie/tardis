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
| `audio::activity` (silence vs speech-like classification) | `tests` in `src/audio/activity.rs` | `cargo run -- mic`, `mock-transcribe`, `mock-translate`, `mock-transcribe-file output/chunks/chunk_001.wav` |
| `audio::chunker` (chunk-size + drain helpers) | `tests` in `src/audio/chunker.rs` | `cargo run -- chunk-test` |
| `audio::chunk_recorder` (chunk filename + f32->i16 WAV helpers) | `tests` in `src/audio/chunk_recorder.rs` | `cargo run -- save-chunks-test` |
| `audio::recorder::record_default_mic_to_wav_for_seconds` | n/a (hound + filesystem glue) | `cargo run -- record-5s` |
| `config` constants | n/a (value-only constants) | consumed by `mic-5s`, `record-5s`, `chunk-test`, `mock-transcribe`, `save-chunks-test`, `mock-transcribe-file`, `mock-translate` |
| `transcription::mock` (`MockTranscriber`) | `tests` in `src/transcription/mock.rs` | `cargo run -- mock-transcribe`, `mock-transcribe-file output/chunks/chunk_001.wav`, `mock-translate` |
| `transcription::file_pipeline` (i16->f32 conversion + WAV-driven mock run) | `tests` in `src/transcription/file_pipeline.rs` | `cargo run -- mock-transcribe-file output/chunks/chunk_001.wav` |
| `transcription::pipeline::run_mock_transcription_test` | n/a (CPAL driver) | `cargo run -- mock-transcribe` |
| `transcription::live_local` (live chunk filename + should_transcribe + status message helpers) | `tests` in `src/transcription/live_local.rs` | `cargo run -- live-local-transcribe [--provider <name>]` |
| `transcription::live_local::run_live_local_transcription_test` | n/a (CPAL + provider driver) | `cargo run -- live-local-transcribe [--provider <name>]` |
| `transcription::transcriber` trait / result types | n/a (abstraction surface, no behavior) | exercised indirectly by `mock-transcribe`, `mock-transcribe-file`, `mock-translate` |
| `translation::mock` (`MockTranslator`) | `tests` in `src/translation/mock.rs` | `cargo run -- mock-translate` |
| `translation::pipeline::run_mock_translate_test` | n/a (CPAL driver) | `cargo run -- mock-translate` |
| `translation::translator` trait / result types | n/a (abstraction surface, no behavior) | exercised indirectly by `mock-translate` |
| `app::config` (defaults + `validate_runtime_config`) | `tests` in `src/app/config.rs` | consumed by `app-mock-flow` |
| `app::events` (AppEvent / AppStatus / payload structs / `status_label` / `is_terminal_status`) | `tests` in `src/app/events.rs` | consumed by `app-mock-flow` |
| `app::state` (AppState mutators + Default) | `tests` in `src/app/state.rs` | consumed by `app-mock-flow` |
| `app::service` (AppService — reuses `MockTranslator`, no CPAL/Docker/fs) | `tests` in `src/app/service.rs` | `cargo run -- app-mock-flow` |
| `app::events` (format_app_event_for_console + app_event_kind) | `tests` in `src/app/events.rs` | consumed by `live-local-transcribe` |
| `app::live_events` (build_transcript_event + build_translation_event + build_error_event) | `tests` in `src/app/live_events.rs` | consumed by `live-local-transcribe` |

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
`audio/chunk_recorder.rs`, `audio/recorder.rs`, `transcription/pipeline.rs`,
and `translation/pipeline.rs`. Keep that duplication honest by updating all
affected sites whenever a new manual command or shared audio-callback rule
lands. If the loop changes again, prefer extracting a shared helper rather
than letting the copies drift.
