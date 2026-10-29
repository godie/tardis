# AGENTS.md

Operational handbook for AI agents working on this repo. Sibling of
`DEVELOPMENT_RULES.md` covers *what* to test; this file covers
*how* to work here: build/test commands, layout, CPAL 0.18 gotchas,
commit style, working conventions, things NOT to do.

## Build + test

```sh
cargo check                                          # fast type-check only (no tests)
cargo test                                           # 219 unit tests on the CLI workspace
cargo run                                            # default = `devices`
cargo run -- devices                                 # list host + I/O devices, exit
cargo run -- mic                                     # capture until Ctrl+C
cargo run -- mic-5s                                  # capture 5 s, exit
cargo run -- record-5s                               # write 5 s WAV to output/mic_test.wav
cargo run -- chunk-test                              # 10 s of capture, 1 s chunks
cargo run -- mock-transcribe                         # 10 s, mock transcript per chunk
cargo run -- save-chunks-test                        # 10 s, save 1 s chunks to output/chunks/
cargo run -- mock-transcribe-file output/chunks/chunk_001.wav
                                                     # run mock transcription over a saved WAV
cargo run -- mock-translate                          # 10 s, mock transcript + translation per chunk
cargo run -- app-mock-flow                           # sync smoke for AppService + AppEvent stream (no mic, no Docker)
cargo run -- live-local-transcribe                    # 10 s, chunk-by-chunk live transcription via AppEvents (default mock-local)
cargo run -- live-local-transcribe --provider local-whisper  # same with Docker provider
cargo run --manifest-path src-tauri/Cargo.toml        # open Tauri UI shell with live transcription + runtime settings
cargo test --manifest-path src-tauri/Cargo.toml       # 28 Tauri shell unit tests (pure helpers + LiveSessionState + SessionCleanupGuard)
```

`cargo run -- <mode>` is the only way to exercise CPAL glue, mic
permission, and disk I/O. Unit tests never touch these. Always run
`cargo check && cargo test` before asking the user to commit.

## File layout

```
src/
  main.rs                  # CLI dispatcher (matches on args.nth(1))
  config.rs                # centralized CLI/audio/pipeline constants
  audio/
    mod.rs                 # re-exports: activity, capture, chunk_recorder, chunker, devices, mic, recorder, volume
    activity.rs            # pure silence/speech-like classifier, 8 tests
    capture.rs             # shared CPAL device resolution + chunk drain (no tests)
    chunk_recorder.rs      # hound chunk saver + pure WAV helpers, 10 tests
    chunker.rs             # pure chunk-size/drain helpers + CPAL capture loop, 13 tests
    devices.rs             # print_device_info (pure print, no tests)
    mic.rs                 # CPAL driver (no tests)
    recorder.rs            # CPAL + hound driver (no tests)
    volume.rs              # pure volume helpers, 14 tests
  transcription/
    mod.rs                 # re-exports: file_pipeline, live_local, local_whisper, mock, mock_local, pipeline, transcriber; ProviderKind + build_provider dispatch, 7 tests
    file_pipeline.rs       # WAV reader + pure i16->f32 helpers, 7 tests
    live_local.rs          # pure helpers + CPAL live local transcription runner, 9 tests
    local_whisper.rs       # LocalWhisperClient: HTTP client for the faster-whisper server, 8 tests
    mock.rs                # mock transcriber logic, 10 tests
    mock_local.rs          # deterministic offline provider for live dev, 5 tests
    pipeline.rs            # live CPAL transcription pipeline (no tests)
    transcriber.rs         # transcription trait + result types
  translation/
    mod.rs                 # re-exports: mock, pipeline, translator
    mock.rs                # mock translator logic, 10 tests
    pipeline.rs            # live CPAL translation pipeline (no tests)
    translator.rs          # translation trait + result types
  app/
    mod.rs                 # re-exports: config, events, live_events, service, settings_store, state, ui_events
    events.rs              # AppStatus + AppEvent stream + payload structs + console formatting, 23 tests
    config.rs              # AppRuntimeConfig + validate_runtime_config + normalize_runtime_config, 36 tests
    live_events.rs         # pure event-construction helpers (build_transcript_event, etc.), 7 tests
    ui_events.rs           # UiAppEvent serializable payloads + app_event_to_ui_event converter, 5 tests
    settings_store.rs      # settings_file_path + atomic save_to_path + load_from_path_or_default + SettingsStoreError, 20 tests
    state.rs               # AppState mutators + Default, 12 tests
    service.rs             # AppService orchestrator (reuses MockTranslator), 15 tests
```

The `app/` layer holds the config and event types that the CLI modes and
the Tauri shell share. It never opens CPAL/Docker/HTTP/filesystem handles
on its own; real captures are wired into it from outside the layer.

When you add a module, mirror an existing one above:
- **Pure helpers** live in their own file with `#[cfg(test)] mod tests`
  so boundary behaviour is unit-testable.
- **CPAL/file I/O glue** that calls them sits next to them with *no*
  tests, exercised only by `cargo run -- <mode>`.

When you change file structure, update `DEVELOPMENT_RULES.md`'s
existing-coverage table in the same commit.

Real transcription backends must land behind the existing
`transcription::transcriber::Transcriber` abstraction so the audio
capture and file-pipeline surfaces stay unchanged. `faster-whisper`,
`whisper.cpp`, and local OpenAI Whisper are reasonable future provider
options, but do not implement them unless the user explicitly asks.

## UI event bridge: Tauri and backend

The backend `AppEvent` stream is the source of truth; the Tauri
shell and the CLI both consume it. The bridge is:

```
CLI / Tauri command
  -> audio/transcription/translation modules
  -> AppEvent (instantiated in src/app/events.rs)
       // CLI prints via format_app_event_for_console
       // Tauri converts via app_event_to_ui_event
  -> UiAppEvent (src/app/ui_events.rs, serde-serializable)
  -> Tauri emit("app-event", payload)
  -> Frontend listener in ui/app.js (switch on payload.kind)
```

- **Pure mapping, fully unit-tested.** `UiAppEvent` and
  `app_event_to_ui_event` live in `src/app/ui_events.rs` with
  five unit tests covering each `AppEvent` variant. Any
  future Tauri-frontend protocol change is a serialisation
  tweak on this struct, **not** a redesign of the backend
  event surface.
- **Provider selection is validated in the backend.**
  `start_live_transcription` takes an optional `AppRuntimeConfig`,
  normalizes and validates it (provider name included) before touching
  shared state, and falls back to `AppRuntimeConfig::default()` when the
  UI sends none. The frontend rebuilds `<select id="provider-select">`
  from `get_supported_transcription_providers`.
- **Session lifecycle is worker-owned.** The worker thread
  spawned by `start_live_transcription` holds a clone of the
  `Arc<LiveSessionState>` managed state and resets both
  `is_running` and `stop_signal` in a `SessionCleanupGuard`
  RAII guard. `Drop` runs on normal return, `Err`, and panic, so
  the next `start_live_transcription` call is unblocked even
  if the worker crashed mid-loop.
- **Tauri runtime + session behavior is manual-tested.** No
  unit test exercises the bound to Tauri's command runtime,
  `AppHandle::emit`, or the cross-thread `Arc<LiveSessionState>`
  ownership pattern. The typed signature enforces part of it (the
  `tauri::State<'_, Arc<LiveSessionState>>` argument forces consumers to
  clone), and `SessionCleanupGuard`'s `Drop` performs the reset; the pure
  parts are unit-tested in isolation. The wiring itself is verified locally by
  `cargo run --manifest-path src-tauri/Cargo.toml` and the
  manual flow described in `docs/tauri-ui-shell.md`.

## CPAL 0.18 gotchas (verified while building the pipeline)

- `cpal::StreamConfig::sample_rate` is `u32` **directly**, not a tuple
  struct. Do not write `config.sample_rate.0`; it fails with
  *"u32 is a primitive type and therefore doesn't have fields"*.
- `cpal::StreamConfig` is `Copy`. Pass it via `*config` to
  `device.build_input_stream(...)`.
- `device.name()` is gone in CPAL 0.18. Use `{}` / `to_string()`
  because `Device` implements `Display`.
- `host.input_devices()` and `host.output_devices()` return
  `Result<Devices, _>`; propagate with `?`, do not `.unwrap()`.
- `cpal::Sample`, `cpal::SizedSample`, and `cpal::FromSample` are at
  the crate root (re-exported from `dasp_sample`). `i16::from_sample(s)`
  is identity for I16, centers for U16 (`(s as i32 - 0x8000) as i16`),
  and clamps+scales for F32. Prefer it over hand-rolled `pcm_to_i16`
  when writing i16 WAVs.
- `cpal::ChannelCount` is `u16`; `cpal::SampleRate` is a plain `u32`
  (not a tuple struct here).
- The audio callback runs on the OS audio thread. Avoid allocation,
  long mutex holds, and `println!` inside the callback. Pure
  decisions should live outside the callback; pass them as a closure
  if they need chunked data.
- Hold the `Arc<Mutex<…>>` lock only long enough to push/drain. Drop
  the `cpal::Stream` before locking for the final drain so the callback
  cannot race with the writer.

## Commit style

Conventional Commits. Subject ≤ 50 chars (hard cap 72), no trailing
period. Body only for non-obvious "why".

| Type | Use for |
| --- | --- |
| `feat(<scope>)` | new user-facing capability (CLI mode, new module) |
| `refactor(<scope>)` | restructure with no user-visible change |
| `fix(<scope>)` | bug fix |
| `test(<scope>)` | new or corrected unit tests |
| `docs` | doc-only change (`DEVELOPMENT_RULES.md`, `AGENTS.md`) |
| `chore` | tooling / ignore-file / dep bumps |

`<scope>` is conventionally `audio`, `transcription`, or `cli`.
Recent shape: `git log --oneline -10`.

Do not embed "Generated with …" / AI attribution trailers unless the
user has set that convention locally. Don't commit on your own unless
the user explicitly asks.

## Working conventions

- A failing test in the pure-helper surface is a real bug. Don't widen
  the assertion to make it pass. Confirm the behaviour is correct
  first.
- For f32 boundaries (e.g. `avg <= threshold`), do not rely on
  `vec![0.01_f32; 1000]`. The 1000 f32 additions accumulate drift and
  land slightly above 0.01, taking the wrong branch. Use a single-sample
  slab so the round-trip is exact. Canonical fix:
  `transcription::mock::tests::volume_equal_to_threshold_returns_none`.
- For per-chunk drivers, reuse `audio::chunker`'s
  `calculate_chunk_size_samples` / `has_complete_chunk` / `drain_chunk`
  rather than re-deriving them.
- Holding a `Mutex<Instant>` inside the audio callback is fine for a
  CLI demo (zero contention). For production, swap to a lock-free
  channel + printer thread.
- Adding a new module? Mirror an existing row in
  `DEVELOPMENT_RULES.md`'s table in the same commit. Stale tables are
  a bug.
- Adding a settings file format? Reuse `src/app/settings_store::settings_file_path`
  and the `SettingsStoreError` enum so failures surface the same way
  across CLI and Tauri callers. New fields on `AppRuntimeConfig`
  should land in one commit alongside a `padded_*` / `legacy_*` /
  `corrupt_*` test in `src/app/settings_store.rs` so the load path
  can never silently accept a forward-incompatible file.

## Things NOT to do

- `device.name()` is gone in CPAL 0.18. Use `{}` / `to_string()`.
- `config.sample_rate.0` does not compile; `sample_rate` is a plain `u32` in 0.18.
- Don't allocate, print, or block for long inside the audio callback.
- Don't `.unwrap()` `host.input_devices()` / `output_devices()` /
  `device.default_input_config()`. Use `?` with a clear `anyhow!` /
  `Context` chain.
- Don't unit-test CPAL streams, mic hardware, or permissions. Split
  logic into pure helpers, test those, then verify the rest manually
  via `cargo run -- <mode>`.
- Don't commit `output/mic_test.wav`. `.gitignore` excludes `/output`.
- Don't add `Todo: …` code paths, `unwrap()`-filled stubs, or dead
  `#[allow(dead_code)]` on functions that should be removed rather
  than silenced.
- Don't add a new desktop-UI surface, async runtime, or transcription
  provider unless the user explicitly asks. The Tauri shell
  (`src-tauri/`) and the `local-whisper` provider already exist; extend
  them instead of starting parallel paths.
- Don't order `mod audio;` twice in `main.rs`; that produces
  `audio is defined multiple times`.
