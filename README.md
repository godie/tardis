# TARDIS v1

TARDIS is a Rust-first foundation for a future desktop app that listens to audio, transcribes it, and translates it locally. Today the repository ships a working CLI audio pipeline, a file-based local transcription provider backed by `faster-whisper`, mock translation flows, and a mock Tauri shell that previews the intended desktop surface without taking ownership of the audio engine yet.

## Current State

- CLI modes exist for device inspection, microphone capture, WAV recording, chunking, chunk saving, mock transcription, mock translation, and file-based local transcription.
- Live audio capture is implemented with `cpal 0.18`.
- A real local transcription provider exists for WAV files sent to a self-hosted `faster-whisper` HTTP server.
- A second local provider, `mock-local`, exists for deterministic offline development.
- Translation is still mock-only.
- The Tauri shell exposes both mock app-status commands and a file-based transcription command (`transcribe_wav_file_local`) that delegates to the same `local-whisper` provider used by `cargo run -- local-transcribe-file`. Live CPAL capture and real translation are still not wired into the UI.
- `cargo test` currently runs 90 unit tests over pure helper logic, provider dispatch, and pure Tauri command helpers.

## Stack

- Rust 2024
- `cpal` 0.18 for audio device access and microphone capture
- `hound` for WAV read/write
- `reqwest` + `serde` for local HTTP transcription provider calls
- Tauri 2 for the desktop shell

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

## Local Transcription Providers

The provider selected by `local-transcribe-file` implements the shared `transcription::LocalTranscriptionProvider` trait.

| Provider | Selector | What it does |
| --- | --- | --- |
| Local faster-whisper HTTP server | `--provider local-whisper` or omit the flag | Sends a WAV file to `http://localhost:8000/v1/audio/transcriptions` and prints the `text` field from the OpenAI-style JSON response. |
| Deterministic offline stub | `--provider mock-local` | Returns `mock transcript for <basename>` without using the network. |

Start the Docker-backed provider with:

```bash
docker compose -f docker/faster-whisper/docker-compose.yml up
```

Then transcribe a saved chunk:

```bash
cargo run -- local-transcribe-file output/chunks/chunk_001.wav
```

Operator details for that container live in [docker/faster-whisper/README.md](docker/faster-whisper/README.md).

## Tauri UI Shell

The repo now includes a desktop shell in `src-tauri/` plus static frontend assets in `ui/`.

Run it with:

```bash
cargo run --manifest-path src-tauri/Cargo.toml
```

What the shell does today:

- Tracks a mock app status: `Idle`, `Listening`, `Stopped`
- Returns a mock transcript string
- Returns a mock translation string
- Lets the frontend preview target-language label changes
- "Local WAV Transcription" card: validate a path, call `transcribe_wav_file_local`, surface the transcript or a user-facing error. This is file-based only and assumes the local faster-whisper Docker container is running.

What it does not do yet:

- Open the microphone from the UI
- Stream live chunks into the window
- Run a real translation backend
- Surface provider selection in the desktop UI

More detail lives in [docs/tauri-ui-shell.md](docs/tauri-ui-shell.md).

## Project Layout

```text
src/
  main.rs                  CLI dispatcher
  lib.rs                   Library surface shared by the CLI binary and src-tauri
  config.rs                Centralized constants
  audio/                   CPAL capture, chunking, recording, pure audio helpers
  transcription/           Traits, mocks, file pipeline, local providers
  translation/             Traits, mocks, live mock translation pipeline
src-tauri/                 Tauri shell crate (mock commands + transcribe_wav_file_local)
ui/                        Static frontend assets for the shell
docker/faster-whisper/     Local Docker transcription stack
docs/                      Supplemental project notes
```

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
  -> console output today, Tauri shell later
```

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
