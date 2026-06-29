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

Status: `partial (UI wired to backend AppEvents, panic-safe cleanup)` → `partial (runtime settings panel live in UI + persisted across sessions + session transcript export)`

What exists:

- `UiAppEvent` — serializable frontend-safe payloads with a
  pure [`app_event_to_ui_event`] mapping (`app::ui_events` — 5 unit tests).
- `start_live_transcription` spawns a background thread that captures audio,
  transcribes chunks, and emits `app-event` Tauri events to the frontend.
  Accepts a full `AppRuntimeConfig` from the UI; providers / language
  pair / chunk size / threshold all flow from the settings panel.
- `stop_live_transcription` signals the session to stop cleanly.
- `get_supported_transcription_providers` + `get_default_runtime_config`
  populate the UI on load (no hard-coded provider list in HTML).
- Runtime settings **persist across sessions** to
  `<OS config dir>/tardis/runtime.json` via the `src/app/settings_store`
  pure helpers (atomic write-then-rename, load-returns-default-on-missing,
  normalize-and-validate round trip). New Tauri commands
  `load_runtime_settings` / `save_runtime_settings` resolve the canonical
  OS path through `tauri::Manager::path()`; the UI calls them on init
  and on every committed settings change respectively.
- Session lifecycle is RAII-cleaned: `LiveSessionState` is registered as
  `Arc<...>` with Tauri and a [`SessionCleanupGuard`] resets `is_running`
  and `stop_signal` on worker exit (success, error, **or panic**) — so a
  fast Start→Stop→Start cycle is unblocked even if the worker crashed.
  12 unit tests cover the pure `LiveSessionState` helpers and the
  panic-safety path via `std::panic::catch_unwind`.
- `[`src/transcription/live_local::run_live_local_transcription_with_config_and_events`]`
  is the canonical live runner; translation uses
  `config.source_language` / `config.target_language` (no hardcoded en/es).

Gaps:

- Settings persistence is **runtime-only**: each user-level OS config
  dir gets one entry, but there is no first-run "remember my last
  session?" prompt, no import/export, and no schema-version field for
  future migrations.
- Translation is still mock-only.
- No system audio capture.
- No streaming/partial transcripts.
- The multithreaded race test for Start/Stop + CleanupGuard is not
  present yet — the panic-safety test is single-threaded only.
- **Session transcript export** is shipped: the backend accumulates
  transcript + translation events into a [`TranscriptSession`]
  ([`tardis::app::session`]) inside `LiveSessionState::current_session`,
  and the UI exposes JSON / plain-text export via
  `export_current_session_json` / `export_current_session_text`.
  Pure helpers in [`tardis::app::session_export`] (JSON / text /
  filename) and file-bound writers in [`tardis::app::session_store`]
  are unit-tested in isolation; the file I/O is manually verified via
  `cargo run -- session-export-demo`, which writes
  `output/sessions/session_demo.{json,txt}`.
  No audio is persisted — only the recognised text, the per-chunk
  provider + language metadata, and the session id / start / end
  stamps.

Next:

- Add a real translation provider behind the `Translator` trait.
- Surface real translation events through the Tauri event stream.
- Add a multithreaded Start/Stop race test to harden the cleanup.
- Add a top-level `schema_version` to the persisted file so future
  `AppRuntimeConfig` field additions can migrate cleanly without
  silently deserializing corrupt data.

Later:

- Add explicit permission and recording-state UX before any release candidate.

## 5. Improve Operability And Contributor Workflow

Status: `partial` → `partial (CI added)`

What exists:

- `README.md`, `AGENTS.md`, and `DEVELOPMENT_RULES.md` document the testing split and command surfaces.
- The Docker provider has its own operational README.
- The repo follows Conventional Commits.
- GitHub Actions CI runs `cargo fmt --check`, `cargo check`, `cargo test`, and Tauri `cargo check` on every PR and push to `main`.
- `README.md` Quickstart mirrors the [`ui-tests` job in `.github/workflows/ci.yml`](.github/workflows/ci.yml) step-by-step — Node pin, `npm ci`, Playwright install, `npm run test:e2e` — with each step cross-referencing its CI equivalent.

Gaps:

- No release workflow (packaging, versioning, binaries).
- No Docker CI for the faster-whisper provider.
- `cargo clippy -- -D warnings` has 4 pre-existing warnings (unnecessary cast, collapsible ifs) that need fixing before clippy can be added to CI.

Next:

- Keep documentation aligned with real commands and shipped modules whenever structure changes.
- Add a release workflow only when the Tauri shell reaches a shippable milestone.

Later:

- Generate release artifacts — see [Generating a release](#generating-a-release) for the TODO list. Once the workflow exists, TARDIS stops being a local-only spike.

## Recommended Execution Order

1. Build the shared backend orchestration layer.
2. Route live transcription through a real provider.
3. Wire the Tauri shell to backend events and controls.
4. Upgrade translation from mock-only to real-provider capable.
5. Add CI and contributor ergonomics around the stabilized architecture.

That order keeps the codebase honest: one backend, one UI shell, and one provider model instead of separate one-off paths for CLI, file transcription, and desktop preview.
