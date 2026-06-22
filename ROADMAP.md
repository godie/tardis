# TARDIS Roadmap

TARDIS is no longer just a CLI spike. The repo now has four meaningful surfaces:

- A working Rust CLI for audio capture and chunk-oriented experiments
- A real file-based local transcription provider via self-hosted `faster-whisper`
- Mock translation flows that prove the end-to-end shape
- A mock Tauri shell that defines the desktop integration target

This roadmap reflects that actual state and focuses on the shortest path to a coherent local desktop product.

Status legend: `done` = shipped, `partial` = real groundwork exists but the user-facing loop is incomplete, `next` = immediate target, `later` = important but not yet first priority.

## 1. Stabilize The Backend Contract

Status: `partial` → `partial (orchestration layer shipped)`

What exists:

- `audio::*` already covers device listing, microphone capture, chunking, chunk saving, and WAV recording.
- `transcription::transcriber::Transcriber` exists for live chunk processing.
- `transcription::LocalTranscriptionProvider` exists for provider-swappable file transcription.
- `translation::*` already proves the translation stage with mock implementations.
- `src/app/` introduces the app-facing orchestration boundary: `AppService`, `AppState`, `AppRuntimeConfig`, and a typed `AppEvent` stream. `cargo run -- app-mock-flow` exercises it end-to-end against `MockTranslator` without CPAL/Docker/fs. Pure-logic coverage (`config`, `events`, `state`, `service` modules) totals ~57 new unit tests.

Gaps:

- `src/app/` is text-input only (`run_mock_text_flow`); connecting it to live microphone capture is the next integration step.
- `AppEvent` consumers today are the CLI smoke command and (eventually) the Tauri shell; no event sink yet.

Next:

- Wire live `audio` capture chunks into `AppService` so a real-time transcript event stream surfaces through the same `AppEvent` boundary already used by mock flows.
- Surface provider selection through `AppRuntimeConfig` and propagate it from the UI / CLI flag to the live provider picker.
- Keep pure decision helpers unit-tested and leave CPAL wiring on the manual CLI path.

## 2. Turn Real Transcription Into A Live Path

Status: `partial` → `partial (live chunk-by-chunk path shipped)`

What exists:

- `local-transcribe-file` can already send a WAV file to a local `faster-whisper` server.
- `mock-local` already proves provider swapping without Docker.
- `live-local-transcribe` captures from the default microphone, splits audio into chunks, and sends each speech-like chunk through a selected provider via a temporary WAV file. Default provider is `mock-local` so the command works without Docker.

Gaps:

- The live path is chunk-by-chunk via temporary WAV files, not true streaming — the audio callback delivers samples but the provider interface is still file-based.
- There is no streamed or true-streaming provider bridge from microphone capture to the local backend.

Next:

- Keep `mock-transcribe` as the zero-dependency validation path.
- Preserve the current `--provider` selection model so adding `whisper.cpp` later does not change the CLI shape.
- Consider a true streaming provider interface (bytes/chunks instead of files) once the file-based chunk path is validated.

## 3. Replace Mock Translation With A Real Translation Boundary

Status: `partial`

What exists:

- `translation::translator::Translator` and the mock translation pipeline already define the handoff point.
- The CLI already proves chunk-by-chunk transcription-to-translation flow.

Gaps:

- Translation is still string formatting, not language-aware transformation.
- There is no source-language detection or provider routing yet.

Next:

- Add a provider-backed translator behind the existing trait.
- Decide whether the first real path is local, cloud-backed, or deferred until transcription quality is acceptable.
- Keep the mock translator for deterministic tests and offline UI work.

Later:

- Add session-level language preferences and source-language detection.
- Support translation policy per target language instead of one fixed demo path.

## 4. Connect The Tauri Shell To The Real Backend

Status: `partial`

What exists:

- `src-tauri/` opens a desktop window and exposes mock commands for app status, transcript text, and translation text.
- `ui/` already gives the project a concrete desktop interaction model.
- `src-tauri` depends on `tardis` as a path dependency so the shell and the CLI binary share one source tree.
- File-based UI transcription is wired: the `transcribe_wav_file_local` Tauri command delegates to `tardis::transcription::build_provider("local-whisper")` and the frontend exposes a "Local WAV Transcription" card with path input, button, status pill, transcript panel, and error display. Pure helpers (`validate_wav_path_input`, `normalize_local_transcription_error`) are unit-tested.

Gaps:

- The shell does not open CPAL streams.
- No transcript or translation events flow from Rust into the window.
- Provider selection is hard-coded to `local-whisper` in the UI; selecting `mock-local` from the desktop still requires the CLI.

Next:

- Replace the mock start/stop commands with calls into the shared backend orchestration layer.
- Emit transcript and translation updates as Tauri events instead of pulling static strings.
- Surface provider selection in the Local WAV Transcription card so the desktop can swap between `local-whisper` and `mock-local` without a CLI fallback.
- Keep the frontend shell thin; the backend remains the source of truth.

Later:

- Add session controls and persisted settings.
- Add explicit permission and recording-state UX before any release candidate.

## 5. Improve Operability And Contributor Workflow

Status: `partial`

What exists:

- `README.md`, `AGENTS.md`, and `DEVELOPMENT_RULES.md` document the testing split and command surfaces.
- The Docker provider has its own operational README.
- The repo follows Conventional Commits.

Gaps:

- No CI workflow is visible in the repo.
- There is no single contributor checklist beyond the handbook documents.

Next:

- Add CI for `cargo check` and `cargo test`.
- Keep documentation aligned with real commands and shipped modules whenever structure changes.

Later:

- Add release notes or milestone tracking once the live backend and Tauri integration start moving together.

## Recommended Execution Order

1. Build the shared backend orchestration layer.
2. Route live transcription through a real provider.
3. Wire the Tauri shell to backend events and controls.
4. Upgrade translation from mock-only to real-provider capable.
5. Add CI and contributor ergonomics around the stabilized architecture.

That order keeps the codebase honest: one backend, one UI shell, and one provider model instead of separate one-off paths for CLI, file transcription, and desktop preview.
