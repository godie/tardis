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
- Mock-only controls (preserved for dev convenience):
  - `get_app_status() -> String`
  - `start_mock_listening() -> String`
  - `stop_mock_listening() -> String`
  - `get_mock_transcript() -> String`
  - `get_mock_translation() -> String`
- Live transcription commands:
  - `start_live_transcription(provider: Option<String>) -> Result<String, String>`
    Spawns a background thread that captures audio, transcribes
    chunks, and emits `app-event` Tauri events (kind: `status`,
    `transcript`, `translation`, `error`) to the frontend.
  - `stop_live_transcription() -> Result<String, String>`
    Signals the active session to stop; the backend emits a
    final `status: stopped` event and the thread exits.
  - `get_supported_providers() -> Vec<String>`
    Returns `["mock-local", "local-whisper"]`.
- File-based local transcription:
  - `transcribe_wav_file_local(file_path: String) -> Result<String, String>`
    Delegates to `tardis::transcription::build_provider("local-whisper")`
    so the same client served by `cargo run -- local-transcribe-file`
    is reachable from the UI without duplicating the HTTP layer.
- UI event payloads (serializable):
  - `tardis::app::ui_events::UiAppEvent` — flat struct emitted as
    `app-event` to the frontend.
  - `tardis::app::ui_events::app_event_to_ui_event` — pure converter
    from backend `AppEvent` to `UiAppEvent`.
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

## Manual flow — live transcription from the UI

1. **Open the Tauri UI** (Option 1 or 2 above).

2. **Select a provider** in the "Transcription provider" dropdown.
   - `mock-local` works without Docker (default, deterministic).
   - `local-whisper` requires the Docker container running.

3. **Click Start Listening**. The status pill changes to
   `Listening`. Speak into the default microphone.

4. **Watch the transcript panel** — each speech-like chunk is
   transcribed via the selected provider and the result appears
   as a Tauri event. The translation panel updates with mock
   translations.

5. **Click Stop**. The status pill changes to `Stopped` and the
   session ends cleanly.

Expected behavior with `mock-local`:
- Transcript: `mock transcript for live_chunk_NNN.wav`
- Translation: `[mock es] mock translation: "..."`

Expected behavior with `local-whisper`:
- Transcript: real faster-whisper output for each chunk.
- If Docker is not running, an error event is emitted.

## Manual flow — transcribe a WAV from the UI

This flow reuses the existing
`cargo run -- save-chunks-test` + `cargo run -- local-transcribe-file`
stack; the UI is a thin caller of the same Rust provider.

1. **Start the Docker service** (loopback only):

   ```bash
   docker compose -f docker/faster-whisper/docker-compose.yml up
   ```

2. **Generate WAV chunks** from the default microphone:

   ```bash
   cargo run -- save-chunks-test
   ```

3. **Open the Tauri UI** (Option 1 or 2 above).

4. **Transcribe a chunk from the UI**:
   - Path input pre-filled with `output/chunks/chunk_001.wav`.
   - Click `Transcribe File`.

5. **Same path via the CLI** for verification:

   ```bash
   cargo run -- local-transcribe-file output/chunks/chunk_001.wav
   ```

## What is live (connected)

- `Start Listening` spawns a background thread that captures audio
  from the default microphone, chunks it, and transcribes speech-like
  chunks through the selected provider.
- `Stop` signals the background thread to stop; the session exits
  cleanly.
- Backend `AppEvent`s are converted to serializable `UiAppEvent`
  payloads and emitted as Tauri `app-event` events to the frontend.

## What is mocked (preserved for dev reference)

- The original `start_mock_listening` / `get_mock_transcript` /
  `get_mock_translation` commands still exist for smoke-testing the
  UI without a microphone.
- Transcript panel (mock flow): `mock transcript: speech detected`
- Translation panel (mock flow): `[mock es] mock transcript: speech detected`

## What is connected (file-based only)

- `Transcribe File` button: validates the path, calls
  `transcribe_wav_file_local`, surfaces the transcript or a
  user-facing error.
- No microphone capture from the UI.
- No system audio capture from the UI.
- No async runtime — the command runs synchronously on Tauri's IPC
  thread and returns `Result<String, String>`.

## What is not connected yet

- No system audio capture from the UI.
- Translation is still mock-only (the backend uses `MockTranslator`).
- No real cloud or remote transcription providers.
- No streaming/partial transcripts — each chunk is transcribed as a
  whole via a temporary WAV file.

## Next integration points

1. Add a real translation provider behind the `Translator` trait.
2. Add system audio capture support.
3. Add session controls and persisted settings.
4. Add explicit permission and recording-state UX.
