# TARDIS v1 Tauri UI Shell

This branch adds a **mock-only** Tauri desktop shell for `tardisv1`.
It does **not** start CPAL microphone capture, does **not** call any
real transcription backend, and does **not** alter the existing CLI
audio or transcription pipeline.

## What was added

- `src-tauri/` — isolated Tauri Rust crate for the desktop shell
- `ui/` — static HTML/CSS/JS frontend loaded by the Tauri window
- Mock commands only:
  - `get_app_status() -> String`
  - `start_mock_listening() -> String`
  - `stop_mock_listening() -> String`
  - `get_mock_transcript() -> String`
  - `get_mock_translation() -> String`

## How to run the UI

### Option 1: run the Tauri binary directly

From the repo root:

```bash
cargo run --manifest-path src-tauri/Cargo.toml
```

This opens the `TARDIS v1` window and loads the static frontend from `ui/`.

### Option 2: use the standard Tauri CLI workflow

If you want the standard Tauri dev command, install the CLI first:

```bash
cargo install tauri-cli --locked --version "^2"
```

Then run:

```bash
cd src-tauri
cargo tauri dev
```

This follows Tauri's standard `frontendDist` development flow.

## What is mocked

- `Start Listening` only flips the in-app status to `Listening`
- `Stop` only flips the in-app status to `Stopped`
- Transcript panel shows:
  - `mock transcript: speech detected`
- Translation panel shows:
  - `[mock es] mock transcript: speech detected`
  - or the same shape with the selected target code in the frontend preview

## What is not connected yet

- No real microphone capture from the UI
- No CPAL stream lifecycle in the Tauri shell
- No live chunking or streaming updates
- No file-based transcription from the UI
- No real transcription providers
- No real translation providers

## Next integration points

1. Replace the mock start/stop commands with a thin UI-facing app state that can call the existing Rust capture pipeline without moving audio logic into the frontend crate.
2. Expose transcript and translation updates as events or channels from Rust to the webview once live chunk processing is connected.
3. Route provider selection through the existing transcription abstraction instead of binding the UI directly to one backend.
4. Keep CLI verification in place so `cargo run -- <mode>` remains the hardware test surface while the UI stays a shell over the same backend logic.
