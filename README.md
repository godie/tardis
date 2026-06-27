# TARDIS v1

TARDIS is a Rust-first foundation for a future desktop app that listens to audio, transcribes it, and translates it locally. Today the repository ships a working CLI audio pipeline, a file-based local transcription provider backed by `faster-whisper`, mock translation flows, and a Tauri shell that drives the same backend from a desktop UI.

## Current State

- CLI modes exist for device inspection, microphone capture, WAV recording, chunking, chunk saving, mock transcription, mock translation, and file-based local transcription.
- Live audio capture is implemented with `cpal 0.18`.
- A real local transcription provider exists for WAV files sent to a self-hosted `faster-whisper` HTTP server.
- A second local provider, `mock-local`, exists for deterministic offline development.
- Translation is still mock-only.
- The Tauri shell supports live microphone transcription via `start_live_transcription` / `stop_live_transcription` commands. Backend `AppEvent`s are converted to serializable `UiAppEvent` payloads and emitted as Tauri events to the frontend. Provider selection is available in the UI (`mock-local` — no Docker; `local-whisper` — requires Docker). Translation is still mock-only.
- The app-facing orchestration layer (`src/app/`) is in place: `AppService` + `AppState` + `AppRuntimeConfig` + a typed `AppEvent` stream. This is the future shared boundary between the CLI modes and the Tauri shell. Reaching it today requires `cargo run -- app-mock-flow` (sync, no microphone, no Docker).
- The runtime settings selected in the UI (provider, language pair, chunk duration, volume threshold) **persist across sessions** in `<OS config dir>/tardis/runtime.json` via the `src/app/settings_store` pure helpers (atomic write-then-rename, load-returns-default-on-missing, normalize-and-validate round trip). See [Persistence](#persistence) below.
- `cargo test` runs ~219+ unit tests over pure helper logic, provider dispatch, the `src/app/` layer, the `settings_store` persistence layer, and pure Tauri command helpers.

## Stack

- Rust 2024
- `cpal` 0.18 for audio device access and microphone capture
- `hound` for WAV read/write
- `reqwest` + `serde` + `serde_json` for local HTTP transcription provider calls and runtime-settings persistence
- Tauri 2 for the desktop shell (uses `tauri::Manager::path()` for the canonical OS config dir)

## Quick Start

```bash
cargo check
cargo test
cargo run
```

`cargo run` defaults to `devices`.

## CLI Modes

| Command | Purpose | Auto-exit |
| --- | --- | --- |
| `cargo run -- devices` | Print host, input devices, output devices, and defaults. | Yes |
| `cargo run -- mic` | Capture from the default microphone continuously. | No |
| `cargo run -- mic-5s` | Capture from the default microphone for 5 seconds. | Yes |
| `cargo run -- record-5s` | Record 5 seconds to `output/mic_test.wav`. | Yes |
| `cargo run -- chunk-test` | Capture 10 seconds and print 1-second chunk metadata. | Yes |
| `cargo run -- save-chunks-test` | Capture 10 seconds and save each 1-second chunk under `output/chunks/`. | Yes |
| `cargo run -- mock-transcribe` | Capture, classify chunks, and print mock transcript results. | Yes |
| `cargo run -- mock-transcribe-file output/chunks/chunk_001.wav` | Run the mock transcriber over a saved WAV chunk. | Yes |
| `cargo run -- mock-translate` | Capture, mock-transcribe, then mock-translate each chunk. | Yes |
| `cargo run -- local-transcribe-file [--provider <name>] <path>` | Send a WAV file to a local transcription provider and print plaintext output. | Yes |
| `cargo run -- live-local-transcribe [--provider <name>]` | Chunk-by-chunk live transcription from the default microphone. Emits typed `AppEvent`s (`StatusChanged`, `Transcript`, `Translation`, `Error`) to the console via `format_app_event_for_console` — the same event stream the Tauri shell will consume. Default provider is `mock-local` (no Docker). Silence chunks are skipped. This is **not** true streaming. | Yes |
| `cargo run -- app-mock-flow` | Sync smoke test of the `app` orchestration layer: `start_listening_mock` &rarr; `run_mock_text_flow("mock transcript: speech detected")` &rarr; `run_mock_text_flow("")` (silent-skip demo) &rarr; `stop_listening`. Prints every emitted event and the final state. No microphone, no Docker, no fs. | Yes |
| `cargo run -- settings-default` | Print the default `AppRuntimeConfig` as pretty JSON. Mirrors the file the Tauri shell writes on first save, so it can be piped into a fresh `runtime.json`. No FS writes, no microphone, no Docker. | Yes |
| `cargo run -- settings-validate <path>` | Load + validate a settings JSON file at `<path>`. Prints the resolved values and exits `0` on success or `1` with the underlying `SettingsStoreError` on parse / validation / I/O failure. Mirrors the `load_runtime_settings` Tauri command. | Yes |

## Local Transcription Providers

The provider selected by `local-transcribe-file` implements the shared `transcription::LocalTranscriptionProvider` trait.

| Provider | Selector | What it does |
| --- | --- | --- |
| Local faster-whisper HTTP server | `--provider local-whisper` or omit the flag (for `local-transcribe-file`) | Sends a WAV file to `http://localhost:8000/v1/audio/transcriptions` and prints the `text` field from the OpenAI-style JSON response. |
| Deterministic offline stub | `--provider mock-local` (default for `live-local-transcribe`) | Returns `mock transcript for <basename>` without using the network. |

Start the Docker-backed provider with:

```bash
docker compose -f docker/faster-whisper/docker-compose.yml up
```

Then transcribe a saved chunk:

```bash
cargo run -- local-transcribe-file output/chunks/chunk_001.wav
```

Or run chunk-by-chunk live transcription (no Docker needed with the default `mock-local` provider):

```bash
cargo run -- live-local-transcribe
cargo run -- live-local-transcribe --provider local-whisper
```

Operator details for that container live in [docker/faster-whisper/README.md](docker/faster-whisper/README.md).

## Tauri UI Shell

The repo now includes a desktop shell in `src-tauri/` plus static frontend assets in `ui/`.

Run it with:

```bash
cargo run --manifest-path src-tauri/Cargo.toml
```

What the shell does today:

- **Session settings panel**: pick a `transcription_provider`, `source_language`, `target_language`, `chunk_duration_ms`, and `volume_threshold` before pressing **Start Listening**. Settings are pre-populated from a persisted `runtime.json` if one exists, otherwise from `AppRuntimeConfig::default()` ([`src/app/config.rs`](src/app/config.rs)). The provider dropdown is rebuilt from the backend's supported-provider list so a future provider shows up without touching the HTML. The settings lock while a session is in progress.
- **Live transcription**: Click Start Listening to capture audio from the default microphone, chunk it using the requested chunk size, classify each chunk with the requested threshold, and transcribe speech-like chunks through the selected provider. Transcript and translation events flow into the window via Tauri events. Click Stop to end the session. Translation uses the selected source/target language pair (no hardcoded en/es).
- **Provider selector**: choose `mock-local` (no Docker, deterministic) or `local-whisper` (requires Docker).
- Default `transcription_provider` is `mock-local`; `local-whisper` requires the Docker container to be running.
- Mock-only controls preserved for dev reference: `start_mock_listening`, `get_mock_transcript`, `get_mock_translation`.
- "Local WAV Transcription" card: validate a path, call `transcribe_wav_file_local`, surface the transcript or a user-facing error.

What it does not do yet:

- System audio capture
- Run a real translation backend
- Streaming/partial transcripts
- Recording / permission UX before any release candidate

More detail lives in [docs/tauri-ui-shell.md](docs/tauri-ui-shell.md).

## Persistence

The Tauri shell persists session settings to `<OS config dir>/tardis/runtime.json` so the same provider / language pair / chunk size / threshold come back next launch:

- Linux: `$XDG_CONFIG_HOME/tardis/runtime.json` (typically `~/.config/tardis/runtime.json`).
- macOS: `~/Library/Application Support/tardis/runtime.json`.
- Windows: `%APPDATA%\tardis\runtime.json`.

Pure helpers live in [`src/app/settings_store.rs`](src/app/settings_store.rs) — atomic write (write-then-rename via `<path>.tmp`), load-returns-default-on-missing, normalize-and-validate round-trip. The Tauri commands `load_runtime_settings` / `save_runtime_settings` resolve the OS path via `tauri::Manager::path()` and delegate to the pure helpers; the UI calls `load_runtime_settings` on init (falling back to `get_default_runtime_config` if the file is missing or unreadable) and `save_runtime_settings` on every committed settings change.

CI does **not** verify the actual filesystem path on a developer machine — only the pure round-trip behaviour in unit tests. The first-run contract is documented in [docs/tauri-ui-shell.md](docs/tauri-ui-shell.md).

## Project Layout

```text
src/
  main.rs                  CLI dispatcher
  lib.rs                   Library surface shared by the CLI binary and src-tauri
  config.rs                Centralized constants
  audio/                   CPAL capture, chunking, recording, pure audio helpers
  transcription/           Traits, mocks, file pipeline, local providers, live-local pipeline
  translation/             Traits, mocks, live mock translation pipeline
  app/                     App-facing orchestration layer (CLI + Tauri shared boundary)
    events/                AppStatus + AppEvent stream + payload structs
    config/                AppRuntimeConfig + validate_runtime_config + normalize_runtime_config + supported providers
    settings_store/        Pure load/save to <OS config dir>/tardis/runtime.json (atomic write, normalize-and-validate round trip)
    state/                 AppState (status + config + last_transcript/translation)
    service/               AppService orchestrator (start_listening_mock, stop_listening, run_mock_text_flow)
src-tauri/                 Tauri shell crate (mock commands + transcribe_wav_file_local + load_runtime_settings + save_runtime_settings)
ui/                        Static frontend assets for the shell
docker/faster-whisper/     Local Docker transcription stack
docs/                      Supplemental project notes
```

The `src/app/` layer is the future shared boundary: today it powers
`cargo run -- app-mock-flow`; once the Tauri shell moves past
mock-only controls, it will host the same `AppService` API the CLI
already exercises, end-to-end from microphone capture to UI event
sink.

## CI

GitHub Actions runs on every PR and push to `main`.

| Check | Command |
| --- | --- |
| Formatting | `cargo fmt --check` |
| Compilation | `cargo check` |
| Tauri compilation | `cargo check --manifest-path src-tauri/Cargo.toml` |
| Unit tests | `cargo test` |

CI does **not** run hardware/audio/manual commands (`live-local-transcribe`, `mic`, `record-5s`, etc.) — those remain local validation only. CPAL streams, microphones, Docker providers, and Tauri runtime are not exercised in CI.

## Testing Model

This repo deliberately separates pure logic from hardware glue.

- Unit tests cover pure helpers and provider-selection logic.
- CPAL streams, device permissions, microphones, and filesystem-heavy audio flows are verified through `cargo run -- <mode>`.
- The working conventions are documented in [AGENTS.md](AGENTS.md) and [DEVELOPMENT_RULES.md](DEVELOPMENT_RULES.md).

## Architecture

Current backend flow:

```text
Microphone or WAV file
  -> audio helpers / chunking
  -> activity classification
  -> transcription trait boundary
  -> optional translation trait boundary
  -> console output today, AppEvent stream tomorrow
```

The `AppEvent` stream is the typed event surface emitted by
`AppService` (the new `src/app/` orchestration layer). Today the
CLI's `app-mock-flow` mode prints it directly; the Tauri shell will
relay each event into the webview once it stops being mock-only.

Design constraints that shape the codebase:

- Pure logic should stay testable next to the module that owns it.
- CPAL and file I/O glue should remain small and manually verifiable.
- Real backends should fit behind existing transcription and translation abstractions.
- The Tauri shell should reuse the backend rather than fork its own audio pipeline.

## Related Docs

- [ROADMAP.md](ROADMAP.md)
- [DEVELOPMENT_RULES.md](DEVELOPMENT_RULES.md)
- [docs/tauri-ui-shell.md](docs/tauri-ui-shell.md)
- [docker/faster-whisper/README.md](docker/faster-whisper/README.md)

## Privacy Note

The project handles microphone audio and may later handle system audio. Any production UI should surface explicit recording state, permission handling, and clear user controls before this becomes a real end-user application.
