# TARDIS v1 Tauri UI Shell

This document covers the Tauri desktop shell for `tardisv1`. The
shell hosts both a **mock UI** (status pill / transcript / translation
panels driven by zero-side-effect Rust commands) and a **file-based
local transcription flow** that reuses the existing
`transcription::local_whisper::LocalWhisperClient` against
`docker/faster-whisper/`.

It does **not** start CPAL microphone capture from the UI, does
**not** call any real translation provider, and does **not** alter the
existing CLI audio or transcription pipeline.

## What was added

- `src-tauri/` — isolated Tauri Rust crate for the desktop shell.
- `ui/` — static HTML/CSS/JS frontend loaded by the Tauri window.
- Mock-only controls:
  - `get_app_status() -> String`
  - `start_mock_listening() -> String`
  - `stop_mock_listening() -> String`
  - `get_mock_transcript() -> String`
  - `get_mock_translation() -> String`
- File-based local transcription:
  - `transcribe_wav_file_local(file_path: String) -> Result<String, String>`
    Delegates to `tardis::transcription::build_provider("local-whisper")`
    so the same client served by `cargo run -- local-transcribe-file`
    is reachable from the UI without duplicating the HTTP layer.
- Pure helpers (unit-testable):
  - `validate_wav_path_input(path: &str) -> Result<String, String>`
    Rejects empty/whitespace inputs; returns the trimmed path on
    success.
  - `normalize_local_transcription_error(error: &str) -> String`
    Maps provider error strings to user-facing prefixes while
    preserving the original detail.

## How to run the UI

### Option 1: run the Tauri binary directly

From the repo root:

```bash
cargo run --manifest-path src-tauri/Cargo.toml
```

This opens the `TARDIS v1` window and loads the static frontend from
`ui/`.

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

## Manual flow — transcribe a WAV from the UI

This flow reuses the existing
`cargo run -- save-chunks-test` + `cargo run -- local-transcribe-file`
stack; the UI is a thin caller of the same Rust provider.

1. **Start the Docker service** (loopback only):

   ```bash
   docker compose -f docker/faster-whisper/docker-compose.yml up
   ```

   The first run pulls the image **and** downloads the `base` Whisper
   model (~150 MB). Subsequent runs reuse the cached model under
   `docker/faster-whisper/hf_cache/`. The container exposes
   `POST http://localhost:8000/v1/audio/transcriptions` — exactly the
   endpoint the Rust client targets. See `docker/faster-whisper/README.md`
   for full operator docs (health checks, model sizes, GPU opt-in,
   privacy).

2. **Generate WAV chunks** from the default microphone:

   ```bash
   cargo run -- save-chunks-test
   # writes output/chunks/chunk_001.wav ... chunk_010.wav
   ```

3. **Open the Tauri UI** (Option 1 or 2 above).

4. **Transcribe a chunk from the UI**:
   - Path input already pre-filled with `output/chunks/chunk_001.wav`.
   - Click `Transcribe File`. Status pill shows `Transcribing…`, the
     transcript appears below in the panel, and any provider error is
     surfaced with a user-facing message that still names the original
     error (URL, HTTP status, Docker hint).

5. **Same path via the CLI** for verification:

   ```bash
   cargo run -- local-transcribe-file output/chunks/chunk_001.wav
   ```

The UI command calls the same `LocalWhisperClient` constructor as the
CLI; only the entry point differs.

## What is mocked

- `Start Listening` only flips the in-app status to `Listening`.
- `Stop` only flips the in-app status to `Stopped`.
- Transcript panel shows:
  - `mock transcript: speech detected`
- Translation panel shows:
  - `[mock es] mock transcript: speech detected`
  - or the same shape with the selected target code in the frontend preview.

## What is connected (file-based only)

- `Transcribe File` button: validates the path, calls
  `transcribe_wav_file_local`, surfaces the transcript or a
  user-facing error.
- No microphone capture from the UI.
- No system audio capture from the UI.
- No async runtime — the command runs synchronously on Tauri's IPC
  thread and returns `Result<String, String>`.

## What is not connected yet

- No live microphone capture from the UI (CPAL untouched from the Tauri shell).
- No system audio capture.
- No live chunking or streaming updates.
- No translation from the UI (only the transcription step is wired).
- No real cloud or remote transcription providers — `local-whisper`
  is the only reachable provider today; `--provider mock-local` is
  reserved for CLI offline mode.

## Next integration points

1. Replace the mock start/stop commands with a thin UI-facing app state that can call the existing Rust capture pipeline without moving audio logic into the frontend crate.
2. Expose transcript and translation updates as events or channels from Rust to the webview once live chunk processing is connected.
3. Route provider selection through the existing transcription abstraction instead of binding the UI directly to one backend — a future "Provider" dropdown in the Local WAV Transcription card would enumerate `ProviderKind` variants instead of hard-coding `local-whisper`.
4. Keep CLI verification in place so `cargo run -- <mode>` remains the hardware test surface while the UI stays a shell over the same backend logic.
