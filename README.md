# TARDIS v1

TARDIS is a Rust-first foundation for a future desktop app that listens to audio, transcribes it, and translates it locally. Today the repository ships a working CLI audio pipeline, a file-based local transcription provider backed by `faster-whisper`, mock translation flows, and a Tauri shell that drives the same backend from a desktop UI.

## Current State

- CLI modes exist for device inspection, microphone capture, WAV recording, chunking, chunk saving, mock transcription, mock translation, and file-based local transcription.
- Live audio capture is implemented with `cpal 0.18`.
- A real local transcription provider exists for WAV files sent to a self-hosted `faster-whisper` HTTP server.
- A second local provider, `mock-local`, exists for deterministic offline development.
- Translation is still mock-only.
- The Tauri shell supports live microphone transcription via `start_live_transcription` / `stop_live_transcription` commands. Backend `AppEvent`s are converted to serializable `UiAppEvent` payloads and emitted as Tauri events to the frontend. Provider selection is available in the UI (`mock-local` — no Docker; `local-whisper` — requires Docker). Translation is still mock-only.
- **Session export**: during a live Tauri session the backend accumulates transcript + translation events into a `TranscriptSession` ([`src/app/session.rs`](src/app/session.rs)). After pressing **Stop**, the user can click **Export JSON** / **Export Text** in the UI to write the session to a path (`output/sessions/session_current.json` / `.txt` by default). No audio or WAV chunks are persisted — only the recognised text and session metadata. Two formats: pretty JSON (round-trippable through `serde_json`) and human-readable plain text.
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

Prerequisites: Node 24 (pinned in `.nvmrc` — `nvm install`), and a stable Rust toolchain ≥ 1.85 (required for the `edition = "2024"` workspace).

### Backend

```bash
cargo check
cargo test
cargo run
```

`cargo run` defaults to `devices`.

On Linux, the live-mic modes (`mic`, `mic-5s`, `record-5s`, `chunk-test`, `save-chunks-test`, `mock-transcribe`, `mock-translate`, `live-local-transcribe`) also need `libasound2-dev` at build time via the `cpal` ALSA backend. The [`apt-get` block under Tauri UI Shell Setup](#tauri-ui-shell-setup) installs it alongside the Tauri GUI deps.

### End-to-end UI tests (Playwright)

This block mirrors the [`ui-tests` job in `.github/workflows/ci.yml`](.github/workflows/ci.yml) step-by-step. The CI job has four steps after `actions/checkout@v4`: `actions/setup-node@v4` (with `node-version: '24'` plus `cache: 'npm'`), `actions/cache@v4` of `~/.cache/ms-playwright`, `install playwright + chromium` (runs `npm ci && npx playwright install --with-deps chromium`), and `run playwright tests`. Locally the browser cache is implicit (Playwright writes to `~/.cache/ms-playwright` by default) and the npm cache is implicit (npm caches in the user-level cache).

Run once per fresh checkout.

1. **Node version** — switch to the pinned Node. Equivalent to the `setup-node` step (`node-version: '24'`, `cache: 'npm'`):

   ```bash
   nvm use 24
   ```

2. **JS dependencies** — clean install from the lockfile. Equivalent to the `npm ci` half of the `install playwright + chromium` step:

   ```bash
   npm ci
   ```

3. **Playwright browser + Linux system deps** — equivalent to the `npx playwright install --with-deps chromium` half of the same step. On Linux the `--with-deps` half requires `sudo` (the `apt-get` libs are shared with the Tauri shell's GUI deps listed in [Tauri UI Shell Setup](#tauri-ui-shell-setup)):

   ```bash
   npx playwright install --with-deps chromium
   ```

4. **Run the e2e suite** — equivalent to the `run playwright tests` step:

   ```bash
   npm run test:e2e
   ```

> **Note**: until real specs ship (in the upcoming session-export feature PR), step 4 exits 0 thanks to the `--pass-with-no-tests` flag the `test:e2e` script carries; drop the flag — or move it into a future `playwright.config.js` as `passWithNoTests: true` — on the first PR that lands specs.

### Run the desktop UI (Tauri)

The Tauri shell bundles the same backend you just built into a desktop window with a settings panel, a live transcript / translation feed, and a JSON / Text export panel. Default workflow (works without extra tools):

```bash
cargo run --manifest-path src-tauri/Cargo.toml
```

On Debian / Ubuntu, install the system deps from [Tauri UI Shell Setup](#tauri-ui-shell-setup) before the first run. macOS / Windows / non-Debian Linux do not need that block.

Alternative (Tauri's standard `frontendDist` workflow, livelier devtools + automatic webview reload on `ui/` changes):

```bash
cargo install tauri-cli --locked --version "^2"
cd src-tauri && cargo tauri dev
```

Both commands open the **TARDIS v1** window. Pick a provider, click **Start Listening**, speak — see [docs/tauri-ui-shell.md](docs/tauri-ui-shell.md) for the byte-accurate manual checklist.

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
| `cargo run -- session-export-demo` | Create a fixture `TranscriptSession` (one transcript + one translation event), then export it as pretty JSON and plain text to `output/sessions/session_demo.{json,txt}`. No microphone, no Docker, no Tauri shell — reusable as a quick smoke test for the entire session/export/file-store pipeline. | Yes |

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

## Tauri UI Shell Setup

The CLI commands in [Quick Start](#quick-start) and [CLI Modes](#cli-modes) compile against `cpal` and `hound` with no GUI dependencies. The Tauri shell additionally pulls in the WebKit / GTK / appindicator toolchain, which Debian / Ubuntu systems do not ship by default. Before the first `cargo run --manifest-path src-tauri/Cargo.toml`, install the system deps that CI installs (the bash block below is verbatim from `.github/workflows/ci.yml` lines 19–28):

```bash
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  libasound2-dev \
  libwebkit2gtk-4.1-dev \
  libgtk-3-dev \
  libappindicator3-dev \
  librsvg2-dev \
  patchelf
```

macOS, Windows, and non-Debian / Ubuntu Linux distributions do not need this block — the WebKit / GTK surface is only required for the Tauri webview to render on Debian / Ubuntu.

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
- "Session Export" card: after a Stop, write the accumulated session to a JSON or plain-text path via `export_current_session_json` / `export_current_session_text` (powered by [`src/app/session_export.rs`](src/app/session_export.rs) + [`src/app/session_store.rs`](src/app/session_store.rs)). A "Show current JSON" button calls `get_current_session_json` for a peek.

The desktop shell exposes the following `#[tauri::command]` entry points. All paths and event types are unit-tested against the documented contracts in `src-tauri/src/lib.rs`; the binding to the Tauri runtime is verified manually ([docs/tauri-ui-shell.md](docs/tauri-ui-shell.md)):

| Command | Purpose |
| --- | --- |
| `start_live_transcription(config: Option<AppRuntimeConfig>)` | Spawns a background worker thread that captures audio from the default mic, transcribes speech-like chunks via the selected provider, and emits an `app-event` Tauri event for every [`AppEvent`]. Returns `"started with provider <name>"`. |
| `stop_live_transcription()` | Signals the active session to stop; the worker thread exits and the [`SessionCleanupGuard`] resets `is_running` + `stop_signal`. Idempotent — a stale Stop on a torn-down session returns `NO_SESSION_RUNNING_ERR`. |
| `transcribe_wav_file_local(path: String)` | File-based local transcription through `local-whisper` (delegates to the same [`LocalWhisperClient`] the CLI uses; "Docker not running?" errors are normalised into a user-facing string). |
| `export_current_session_json(path: String)` | Saves the in-memory [`TranscriptSession`] as pretty JSON at `path`; consumes the snapshot. |
| `export_current_session_text(path: String)` | Saves the in-memory [`TranscriptSession`] as plain text at `path`; consumes the snapshot. |
| `get_current_session_json()` | Read-only JSON render of the current session — does **NOT** consume (a "Show current JSON" peek in the UI). |
| `get_supported_transcription_providers()` | Backend's `["mock-local", "local-whisper"]` list — the UI rebuilds the provider dropdown from this so a future provider shows up without touching the HTML. |
| `get_default_runtime_config()` | Default [`AppRuntimeConfig`] for first-load pre-population. |
| `load_runtime_settings()` | Loads `<OS config dir>/tardis/runtime.json`, falls back to defaults on first run or if the file is missing. |
| `save_runtime_settings(config)` | Atomically writes the validated + normalised config (write-then-rename via `<file>.tmp`). |
Five mock-only controls preserved for dev reference (no microphone, no Docker, no CPAL): `get_app_status()`, `start_mock_listening()`, `stop_mock_listening()`, `get_mock_transcript()`, `get_mock_translation()`.

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

## Generating a release

The desktop shell is **per-OS bundle-ready**: `src-tauri/Cargo.toml` + `src-tauri/tauri.conf.json` configure Tauri 2 such that `cargo tauri build` produces a native bundle for the host it runs on. Cross-platform coverage needs a 3-runner CI matrix; the project does **not** ship binaries yet. Anything below prefaced with `[x]` already ships with `tardis-v0.1.0` (the Node-24-pin / Linux-libasound2 / Tauri-shell-setup / UI-tests-CI-gate milestone at `bf39ea2`); unchecked items are required before the next tag ships.

- [x] **Per-OS bundle config is wired.** `src-tauri/Cargo.toml` + `src-tauri/tauri.conf.json` configure Tauri 2; `cargo tauri build` produces native bundles from the repo root.
- [x] **Linux `apt-get` block** + macOS / Windows / non-Debian Linux note live in [Tauri UI Shell Setup](#tauri-ui-shell-setup); the same block CI installs is what a developer needs locally.
- [ ] **Pick the next tag.** Once the session-export feature lands and a real translation backend is wired in, the natural next milestone is `tardis-v0.2.0` (a bigger bundle than `tardis-v0.1.0`, which marks the Node-24 / linux-libasound2 / Tauri-shell-setup / UI-tests-CI-gate milestone).
- [ ] **Bump version strings.** `Cargo.toml` (`tardis` CLI), `src-tauri/Cargo.toml` (`tardis-ui-shell`), and `src-tauri/tauri.conf.json` (`bundle.identifier`, `bundle.version`, `productName`) must all agree for consistent metadata (CLI binary version, Tauri bundle installer version, AppImage / `/usr/bin/tardis` version). Mismatches do not fail the build but produce inconsistent version strings across artefacts.
- [ ] **Run `cargo build --release` for the CLI binary** (no UI shell — useful for headless / CI smoke users who do not need the desktop app).
- [ ] **Run `cargo tauri build` for the desktop shell** to produce native bundles:
  - `.deb` + `.AppImage` + `.rpm` (Linux — needs `dpkg-deb`, `rpmbuild`, `fakeroot`, `appimagetool`)
  - `.dmg` (macOS — `cargo tauri build` does the disk-image; code signing needs `codesign` + `notarytool` for notarisation)
  - `.msi` + `.exe` (Windows — needs `signtool` for Authenticode signing; WiX for `.msi` already ships with Tauri 2)
  - **Build matrix:** each platform builds on its own host — Tauri 2 has no first-class OS-cross-compile path (the project itself does not adopt `cargo-zigbuild` or `cargo-xwin` workarounds; they remain an opt-in for downstream forks). CI needs three runners (`ubuntu-latest` / `macos-latest` / `windows-latest`).
- [ ] **Code signing + notarisation.**
  - macOS: `codesign --deep --timestamp --options=runtime` + `xcrun notarytool submit --wait` + staple the ticket.
  - Windows: `signtool sign /fd SHA256 /tr http://timestamp.digicert.com` + Authenticode timestamp authority.
  - Linux: GPG `.sig` artifacts sidecar to `.deb` / `.rpm` (`.AppImage` is unsigned by convention; document that).
  - Tauri's `tauri.conf.json` `bundle.macOS.signingIdentity`, `bundle.windows.certificateThumbprint`, and `bundle.beforeBundleCommand` are where these hooks live.
- [x] **GitHub Release workflow** (`.github/workflows/release.yml`): triggers on the canonical `tardis-vX.Y.Z` SemVer tag pattern (the `v*.*.*` legacy form was retired alongside the deprecated `v0.1.0` tag — see [RELEASES.md § Tagging convention](RELEASES.md#tagging-convention) for the namespace rationale), runs the build matrix above, uploads `.deb`, `.dmg`, `.msi`, `.AppImage`, plus checksums (`SHA256SUMS.txt`) to a GitHub Release with auto-generated notes from commits since the prior tag.
- [ ] **Lockfile reproducibility.** `Cargo.lock` commits prevent supply-chain drift across builds; bump it on every release-prep with `cargo update --workspace` and review the diff in the same commit that bumps version strings.
- [ ] **First-run UX + privacy.** Per the [Privacy Note](#privacy-note): visible recording-state indicator, microphone-permission prompt honouring the OS convention, and a "Stop" button reachable in one click — all required before any external distribution.
- [x] **Install / update docs.** A `RELEASES.md` (or a `## Releases` section heading) linking to the GitHub Releases page plus per-platform install walkthrough (`apt install ./tardis.deb`, `brew install --cask tardis`, `winget install tardis`/next-next-finish on Windows) ships next to the binary in the same release commit.

Tracks the bigger goal in ROADMAP §5 "Later" — once a release workflow exists, TARDIS stops being a local-only spike.

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
