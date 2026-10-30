# TARDIS Roadmap

TARDIS has four working surfaces:

- A Rust CLI for audio capture and chunking
- A file-based transcription provider backed by self-hosted `faster-whisper`
- A mock process stage that carries each transcript through to an output line (translate mode) and, from PR 2, a mock synthesize/play loop
- A Tauri desktop shell that runs the same backend

The roadmap tracks what exists and the shortest path to a usable desktop product.

Status legend: `done` = shipped, `partial` = real groundwork exists but the user-facing loop is incomplete, `next` = immediate target, `later` = important but not yet first priority.

## 1. Stabilize The Backend Contract

Status: `partial` → `partial (orchestration layer shipped)`

What exists:

- `audio::*` covers device listing, microphone capture, chunking, chunk saving, WAV recording, and silence endpointing (`audio::endpoint` turns fixed windows into whole utterances).
- `transcription::LocalTranscriptionProvider` is the real backend contract (file in, text out): `LocalWhisperClient` (Docker `faster-whisper`) and `MockLocalProvider` implement it, and the live runner selects between them through `ProviderKind` / `build_provider`.
- `transcription::transcriber::Transcriber` exists for in-process `&[f32]` chunk processing; today only `MockTranscriber` implements it and it drives the mock pipelines (`mock-transcribe*`, `mock-translate`).
- `processor::*` carries the process stage: `Processor` returns `Option<Output>` (translate today, respond later), `build_processor` dispatches, and `MockProcessor` is the deterministic default.
- `synthesis::*` + `audio::player` complete the loop with `Synthesizer` / `Player` contracts and deterministic mocks; `AppEvent::Speech` reports the rendered duration.
- `src/app/` introduces the app-facing orchestration boundary: `AppService`, `AppState`, `AppRuntimeConfig`, and a typed `AppEvent` stream. `cargo run -- app-mock-flow` exercises it end-to-end against `MockProcessor` without CPAL/Docker/fs; `cargo run -- mock-voice-flow` runs the same stages through the `Speech` event. Pure-logic coverage (`config` 36, `events` 26, `state` 12, `service` 15) totals 89 unit tests.
- Provider selection is surfaced end to end: `transcription_provider` on `AppRuntimeConfig` (default `mock-local`, validated against `ProviderKind`), `#provider-select` built from `get_supported_transcription_providers`, and `--provider <name>` on the CLI.

Gaps:

- The live path and the mock path do not share an orchestrator. Live capture deliberately bypasses `AppService`: `AppService`'s module doc forbids it holding a CPAL stream, so the live runner (`transcription/live_local.rs`) owns capture and emits through `app::live_events` instead. Routing capture *into* `AppService` is not the fix; the missing piece is an injection point, because the stage implementations are constructed inside the runners.
- The process stage is injectable now (`build_processor` in the live runner, `AppService::with_processor` in the mock flow) but only `mock` exists behind it, and the config has no field to select another one yet.

Next:

- Introduce injectable stage traits (`Processor`, later `Synthesizer` / `Player`) behind the existing mock implementations, so the live runner and `AppService` stop constructing their own stages. Design and PR order: [`docs/core-pipeline-plan.md`](docs/core-pipeline-plan.md) — needs the maintainer's go-ahead before adding providers or an async runtime.
- Keep pure decision helpers unit-tested and leave CPAL wiring on the manual CLI path.

## 2. Turn Real Transcription Into A Live Path

Status: `partial` → `partial (live utterance-by-utterance path shipped)`

What exists:

- `local-transcribe-file` sends a WAV file to a local `faster-whisper` server.
- `mock-local` proves provider swapping without Docker.
- `live-local-transcribe` captures from the default microphone in fixed windows, runs them through `audio::endpoint` (`hangover_ms` / `min_speech_ms` / `max_utterance_ms`) so whole utterances reach the provider, and sends each utterance through a selected provider via one temporary WAV file. Default provider is `mock-local` so the command works without Docker.

Gaps:

- The live path is utterance-by-utterance via temporary WAV files, not true streaming: the audio callback delivers samples, but the provider interface is still file-based. Utterance boundaries are decided by volume + hangover, not by a real VAD model.
- No streaming provider bridge connects microphone capture to the local backend.

Next:

- Keep `mock-transcribe` as the zero-dependency validation path.
- Preserve the current `--provider` selection model so adding `whisper.cpp` later does not change the CLI shape.
- Consider a true streaming provider interface (bytes/chunks instead of files) once the file-based chunk path is validated.

## 3. Replace Mock Translation With A Real Translation Boundary

Status: `partial`

What exists:

- `processor::processor::Processor` and the mock translate pipeline already define the handoff point.
- The CLI already proves utterance-by-utterance transcription-to-output flow.

Gaps:

- Translation is still string formatting, not language-aware transformation.
- There is no source-language detection or provider routing yet.

Next:

- Add a provider-backed translator behind the existing trait.
- Decide whether the first real path is local, cloud-backed, or deferred until transcription quality is acceptable.
- Keep the mock translator for deterministic tests and offline UI work.

Later:

- Add source-language detection. (Session-level language preferences are shipped: `source_language` / `target_language` on `AppRuntimeConfig`, `#source-language` / `#target-language` in the settings panel, consumed by the live runner with no hardcoded pair.)
- Support translation policy per target language instead of one fixed demo path.

## 4. Connect The Tauri Shell To The Real Backend

Status: `partial (UI wired to backend AppEvents, panic-safe cleanup)` → `partial (runtime settings panel live in UI + persisted across sessions)`

What exists:

- `UiAppEvent`: serializable frontend-safe payloads with a
  pure [`app_event_to_ui_event`] mapping (`app::ui_events`, 5 unit tests).
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
  and `stop_signal` on worker exit (success, error, **or panic**), so a
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
- The process stage is still mock-only: `build_processor` resolves to `mock` because no config field selects a real provider yet.
- No system audio capture.
- No streaming/partial transcripts.
- The multithreaded race test for Start/Stop + CleanupGuard is not
  present yet; the panic-safety test is single-threaded only.
- Session transcript export is not implemented. There is no
  `TranscriptSession` accumulator, no `session_export` / `session_store`
  module, no export Tauri command, and no export control in the UI.
  The shell keeps no session history, so the transcript panel shows
  only the latest chunk. Earlier revisions of this roadmap described
  this feature as shipped; it was never merged.

Next:

- Add the session accumulator (transcript + translation events per
  session) plus JSON / plain-text export, and surface export controls
  in the UI.
- Generalize the process stage to a `Processor` trait (returns `Option<Output>`, so "stay silent" is a normal result) and inject it, then add a real provider in translate mode; see [`docs/core-pipeline-plan.md`](docs/core-pipeline-plan.md).
- Surface real translation events through the Tauri event stream.
- Add a multithreaded Start/Stop race test to harden the cleanup.
- Add a top-level `schema_version` to the persisted file so future
  `AppRuntimeConfig` field additions can migrate cleanly without
  silently deserializing corrupt data.

Later:

- Add explicit permission and recording-state UX before any release candidate.

## 5. Improve Operability And Contributor Workflow

Status: `partial` → `partial (release workflow added)`

What exists:

- `README.md`, `AGENTS.md`, and `DEVELOPMENT_RULES.md` document the testing split and command surfaces.
- The Docker provider has its own operational README.
- The repo follows Conventional Commits.
- GitHub Actions CI runs `cargo fmt --check`, `cargo check`, `cargo test`, and Tauri `cargo check` on every pull request and every push to `master` (the default branch).
- `README.md` Quickstart mirrors the [`ui-tests` job in `.github/workflows/ci.yml`](.github/workflows/ci.yml) step-by-step (Node pin, `npm ci`, Playwright install, `npm run test:e2e`), with each step cross-referencing its CI equivalent.
- [`.github/workflows/release.yml`](.github/workflows/release.yml) is a tag-driven release workflow that publishes `.deb` / `.AppImage` / `.dmg` / `.msi` / `.exe` plus a canonical `SHA256SUMS.txt` to a GitHub Release on every `tardis-vX.Y.Z` SemVer tag, with auto-generated notes from commits since the prior tag. [`RELEASES.md`](RELEASES.md) at the repo root documents the per-platform install walkthrough (`apt install ./<deb>`, `brew install --cask tardis` *projected*, `winget install tardis` *projected*, plus the WiX Next → Next → Finish flow on Windows).

Gaps:

- No Docker CI for the faster-whisper provider.
- `cargo clippy` is not in CI because `-D warnings` would fail: 7 lints on the lib (5 `collapsible_if`, 1 unnecessary cast, 1 `default()` on a unit struct plus an `or_else`/`map_err` pair) and 30 across `--all-targets`, 22 of them `field_reassign_with_default` in test setup.

Next:

- Keep documentation aligned with real commands and shipped modules whenever structure changes.

Later:

- Generate release artifacts; see [Generating a release](README.md#generating-a-release) for the list of remaining steps.

## Recommended Execution Order

1. Build the shared backend orchestration layer.
2. Route live transcription through a real provider.
3. Wire the Tauri shell to backend events and controls.
4. Upgrade translation from mock-only to real-provider capable.
5. Add CI and contributor ergonomics around the stabilized architecture.

The order follows the dependency chain: one backend, one UI shell, and one provider model, instead of separate one-off paths for the CLI, file transcription, and desktop preview.
