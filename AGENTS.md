# AGENTS.md

Operational handbook for agents working in this repo. `DEVELOPMENT_RULES.md`
covers what to test and where coverage lives; this file covers how to work
here: commands, layout, stage contracts, CPAL gotchas, commit style, and the
gate before a PR.

Scope: everything under `tardis/`, including `src-tauri/` and `ui/`. The
roadmap and the PR-by-PR plan live in
[`docs/core-pipeline-plan.md`](docs/core-pipeline-plan.md).

Precedence: an explicit user request beats this file. When a rule here
contradicts the code, the code wins: correct the rule in the same change that
made it wrong.

The stage contracts, the no-async rule, the secrets rules and the validation
matrix are invariants. File names, module inventories and command lists are
current detail, not doctrine. When detail and code disagree, update the detail
instead of bending the code to match it.

## Product direction: one loop, five stages

```
capture audio -> transcribe -> process (LLM) -> synthesize -> play
```

Every use case runs the same five stages. Only `process` differs, and the
difference is configuration, not a separate pipeline:

| Mode | Prompt intent | Output language | When it acts |
| --- | --- | --- | --- |
| `translate` | Restate the segment in the target language, add nothing | target (differs from source) | Every speech segment |
| `respond` | Answer as the host, using session context | same as input | Only when the decision step says so |

Invariants:

- **No async runtime.** The stages are synchronous and object-safe, and
  `reqwest::blocking` is what the providers use. Adding `tokio`, `async-trait`
  or a background executor needs the user's explicit go-ahead, in its own PR.
- **No new provider or desktop surface without a go-ahead.** Extend the
  existing stage traits and the existing Tauri shell; do not start a parallel
  path.
- **`respond` mode never speaks a commitment** (dates, money, agreements,
  opinions). It suggests them to the host instead. Two layers enforce it:
  classify the request, then validate the generated reply.
- **Mocks first.** Every stage has a deterministic mock, so the whole loop runs
  with no microphone, no Docker and no model.

## Build + test

```sh
cargo check                                          # fast type-check only (no tests)
cargo clippy --all-targets -- -D warnings            # lints, same gate CI runs
cargo test                                           # CLI workspace unit tests
cargo run -- devices                                 # list host + I/O devices, exit
cargo run -- mic                                     # capture until Ctrl+C
cargo run -- live-local-transcribe [--provider <name>] [--processor <name>]
cargo run -- app-mock-flow                           # AppService + AppEvent smoke, no mic
cargo run -- mock-voice-flow                         # five-stage mock loop, no mic/Docker/TTS
cargo run -- llm-translate-text "text"               # one LLM round-trip, no mic
cargo run --manifest-path src-tauri/Cargo.toml       # open the Tauri shell
cargo test --manifest-path src-tauri/Cargo.toml      # Tauri command + pure-helper tests
npm run test:e2e                                     # Playwright specs in ui/tests
```

The full surface, including the one-shot capture modes and the settings
helpers, is the CLI table in [`README.md#cli-modes`](README.md#cli-modes) and
the doc header of `src/main.rs`.

What to run before asking the user to commit is the
[validation matrix](DEVELOPMENT_RULES.md#validation-matrix), keyed by the area
you touched.

Test counts belong in the PR body, as evidence of what actually ran, and
nowhere else. In a reader-facing doc or a source comment they drift within a
week and nobody updates them. A PR that touches no code and no tests reports
no count: nothing ran that a number would describe.

The Playwright suite covers the shell's frontend lifecycle (Start sends a full
`AppRuntimeConfig`, the panel locks, Stop releases on the `stopped` event,
`transcript` / `output` / `speech` payloads render). It injects a fake
`window.__TAURI__` bridge, so the Rust commands and the real window stay
manual: `tauri-driver` does not support macOS. Add a spec under `ui/tests/`
when the frontend gains a state transition worth asserting; do not assert on
pixel layout.

CPAL glue and microphone permission are exercised only by
`cargo run -- <mode>`. Disk I/O is not in that bucket: path handling and
record formatting get temp-dir tests where they own logic worth pinning.

## File layout

One Rust crate plus a Tauri crate and a static frontend:

- `src/audio/` capture and output: CPAL drivers, chunking, endpointing, the
  half-duplex gate, the `Player` trait.
- `src/transcription/` the two transcription contracts and their providers
  (`mock-local`, `local-whisper`).
- `src/processor/` the process stage: `Processor`, the mock, the LLM client,
  the prompt.
- `src/synthesis/` the `Synthesizer` trait and its mock.
- `src/app/` config, events, state, orchestration, settings and history.
- `src/main.rs` the CLI dispatcher; `src/config.rs` the shared constants.
- `src-tauri/` the desktop shell: commands, session lifecycle, file paths.
- `ui/` static HTML/JS/CSS plus the Playwright specs.

`DEVELOPMENT_RULES.md` owns the module-by-module coverage table. Do not copy
it here; it goes stale the first week.

When you add a module, mirror an existing one:

- **Pure helpers** live in their own file with `#[cfg(test)] mod tests`, so
  boundary behaviour is unit-testable.
- **Hardware glue** (CPAL, microphone permission) gets no unit tests. Verify
  it with `cargo run -- <mode>`.
- **File I/O** is testable when it owns logic worth pinning: path handling,
  atomic writes, partial appends. `app::settings_store` and the Tauri history
  writer use temp directories to prove it.

## The `app/` layer

`app/` holds the config, event and orchestration types that the CLI and the
Tauri shell share. It never opens CPAL, Docker or HTTP handles, and it never
decides a filesystem path.

Two deliberate exceptions, both taking a path from the caller:

- `settings_store` reads and writes the settings file with an atomic
  write-then-rename.
- `history` formats JSONL records; the Tauri crate owns the file and writes
  them.

Anything else in `app/` that reaches for the filesystem is a bug.

## Two transcription abstractions, not interchangeable

- `transcription::LocalTranscriptionProvider` (`src/transcription/mod.rs`,
  `transcribe(&self, file_path: &str) -> Result<String>`): file in, text out.
  The real backends implement it (`LocalWhisperClient`, `MockLocalProvider`),
  and the live runner calls it through `ProviderKind` / `build_provider`. New
  HTTP or sidecar backends go here.
- `transcription::transcriber::Transcriber` (`transcriber.rs`,
  `transcribe_chunk(&self, chunk_index: usize, samples: &[f32]) ->
  Option<TranscriptionResult>`): in-process sample slice, no container,
  `None` skips the chunk. Only `MockTranscriber` implements it, and it drives
  the mock pipelines and the `mock-transcribe*` modes. It cannot express a
  provider that needs a WAV/HTTP boundary, so do not claim real backends
  "land behind" it.

## Process stage: `Processor`

`processor::processor::Processor` is
`process(&self, text, &ProcessMode) -> anyhow::Result<Option<Output>>`.
Translation is one mode of it; respond mode answers as the host through the
same trait. `Ok(None)` means deliberate silence (empty input, or a decision
rule that says "stay silent"). `Err` is a failure, and the runner emits
`AppEvent::Error` for it. Never collapse those two into one `None`.

- Pick an implementation with `processor::build_processor(name)`, mirroring
  `transcription::build_provider`. `DEFAULT_PROCESSOR_NAME` is the only place
  the default string lives.
- The runner builds the stage and passes it in; `AppService::with_processor`
  is the injection point for the text-only mock flow. Do not construct a
  concrete implementation inside a loop.
- `MockProcessor`'s rendered string is frozen: changing it is a user-visible
  output change and belongs to a provider PR, not a refactor.

## UI event bridge

The backend `AppEvent` stream is the source of truth; the Tauri shell and the
CLI both consume it.

```
CLI / Tauri command
  -> audio / transcription / processor modules
  -> AppEvent (src/app/events.rs)
       // CLI prints via format_app_event_for_console
       // Tauri converts via app_event_to_ui_event
  -> UiAppEvent (src/app/ui_events.rs, serde-serializable)
  -> Tauri emit("app-event", payload)
  -> Frontend listener in ui/app.js (switch on payload.kind)
```

- **The mapping is pure and unit-tested.** `UiAppEvent` and
  `app_event_to_ui_event` carry every variant. A change that only reshapes
  what the UI already shows is a serialisation change on this struct. A new
  domain concept needs a new `AppEvent` variant and a matching `UiAppEvent`,
  which is a deliberate change to the event surface.
- **Provider selection is validated in the backend.**
  `start_live_transcription` takes an optional `AppRuntimeConfig`, normalizes
  and validates it (provider and processor names included) before touching
  shared state, and falls back to `AppRuntimeConfig::default()` when the UI
  sends none. Both dropdowns are rebuilt from the backend's lists.
- **Session lifecycle is worker-owned.** The worker thread holds a clone of
  the `Arc<LiveSessionState>` managed state and resets `is_running` and
  `stop_signal` in a `SessionCleanupGuard`. `Drop` runs on normal return,
  `Err` and panic, so the next start is unblocked even if the worker crashed.
- **The Tauri runtime and the real window are manual-tested.** No unit test
  exercises `AppHandle::emit` or the cross-thread ownership pattern. The pure
  parts are unit-tested; the wiring is verified with
  `cargo run --manifest-path src-tauri/Cargo.toml` and the flow in
  `docs/tauri-ui-shell.md`.

## Secrets and logs

- Provider keys come from the environment (`TARDIS_LLM_API_KEY`). Do not add a
  key to `AppRuntimeConfig`: from there it reaches `runtime.json`, an
  `AppEvent` and a `UiAppEvent` payload. Never put a real key value in a test
  fixture; use an obvious dummy such as `sk-test`.
- Wrap a key in `processor::llm::Secret` and run every error path through
  `redact_secret` before it reaches `stderr` or an event.
- Never print a request header, a full request body, or a raw provider error
  that may echo either.
- Do not log transcripts or rendered output beyond the session history file
  the user already asked for.

## CPAL 0.18 gotchas (verified while building the pipeline)

- `cpal::StreamConfig::sample_rate` is `u32` **directly**, not a tuple struct.
  `config.sample_rate.0` fails with *"u32 is a primitive type and therefore
  doesn't have fields"*.
- `cpal::StreamConfig` is `Copy`. Pass it via `*config` to
  `device.build_input_stream(...)`.
- `device.name()` is gone in 0.18. Use `{}` / `to_string()`; `Device`
  implements `Display`.
- `host.input_devices()` and `host.output_devices()` return
  `Result<Devices, _>`; propagate with `?`.
- `cpal::Sample`, `cpal::SizedSample` and `cpal::FromSample` are at the crate
  root. `i16::from_sample(s)` is identity for I16, centers for U16, and
  clamps+scales for F32; prefer it over hand-rolled `pcm_to_i16`.
- `cpal::ChannelCount` is `u16`; `cpal::SampleRate` is a plain `u32`.
- The audio callback runs on the OS audio thread: no allocation, no
  `println!`, no long mutex holds. Pure decisions live outside the callback.
- Hold the `Arc<Mutex<…>>` lock only long enough to push or drain, and drop
  the `cpal::Stream` before the final drain so the callback cannot race.
- Multichannel devices: fold to mono with `audio::capture::mono_mix_weights`
  before the volume gate. Averaging every channel makes a 16-channel loopback
  device read as silence.

## Commit style

Conventional Commits. Aim for a 50-character subject; 72 is the hard cap that
git and GitHub truncate past. No trailing period. Body only for the "why" the
diff cannot show. If the subject does not fit in 50, the change is probably
two commits.

| Type | Use for |
| --- | --- |
| `feat(<scope>)` | new user-facing capability (CLI mode, module, provider) |
| `refactor(<scope>)` | restructure with no user-visible change |
| `fix(<scope>)` | bug fix |
| `test(<scope>)` | new or corrected tests |
| `docs` | doc-only change |
| `chore` | tooling, ignore files, dependency bumps |

`<scope>` is one of `audio`, `transcription`, `processor`, `app`, `tauri`,
`ui`, `cli`. Recent shape: `git log --oneline -10`.

Do not embed "Generated with …" or AI attribution trailers unless the user
has set that convention locally. Do not commit unless the user asks.

## Working conventions

- A failing test in the pure-helper surface is a real bug. Do not widen the
  assertion to make it pass; confirm the behaviour is correct first.
- f32 boundaries (`avg <= threshold`): assert on exact values, not on
  accumulations. The canonical case is in `DEVELOPMENT_RULES.md`.
- For per-chunk drivers, reuse `audio::chunker`'s
  `calculate_chunk_size_samples` / `has_complete_chunk` / `drain_chunk`
  instead of re-deriving them.
- Holding a `Mutex<Instant>` inside the audio callback is fine for a CLI demo.
  For production, move to a lock-free channel plus a printer thread.
- Any change to the persisted settings format, whether a new field, a rename
  or a migration, reuses `src/app/settings_store::settings_file_path` and
  `SettingsStoreError`, lands the field with `#[serde(default)]`, and adds a
  `legacy_*` plus a `padded_*` / `corrupt_*` test in the same commit.
- Prefer deleting code to adding it. If an abstraction has one implementation
  and no planned second one, it does not need to exist yet.

## Before opening a PR

The gates scale with the change, so a typo fix does not pay for a feature
review.

| Change | Gates |
| --- | --- |
| Docs only | 2 |
| One-line fix, no new surface | 1 and 2 |
| Feature or refactor | all of them |
| More than 20 files | split first, then run this list per PR |

A finding you decide not to fix belongs in the PR body with the reason.

1. **Validation matrix** for the areas you touched
   ([DEVELOPMENT_RULES.md](DEVELOPMENT_RULES.md#validation-matrix)). For code
   or test changes, paste the resulting counts into the PR body next to each
   command. A docs-only PR reports no counts.
2. **Prose pass** with the `no-ai-slop` skill: PR body, commit messages, and
   the doc comments this PR adds. Cut em-dash asides, bold mid-sentence,
   tables that carry three bullets, and claims the diff does not support.
3. **Overengineering pass.** For each new abstraction, ask what breaks if it
   is deleted. If the answer is "nothing", delete it.
4. **SOLID / KISS / YAGNI refactor** of what the PR implements. Single
   responsibility per module, no option or flag nobody asked for, no interface
   with one caller.
5. **`ponytail-review`** over the change, and fix the findings. Review the
   working tree with `git diff`, or the commit if the user already asked for
   one.
6. **`caveman-review`** as a second, independent local review, and fix the
   findings. If the two reviews disagree, say why in the PR.
7. **Atomic commits.** One logical change per commit, conventional subject,
   build green at each commit that others may bisect to.
8. **At most 20 files per PR.** A feature that touches more is split by
   feature or by domain (audio, processor, app, tauri, ui) into stacked PRs
   that each land green. A mechanical rename is the exception, and the PR body
   has to say so. If splitting would leave the base branch unbuildable, stack
   the PRs and merge them in order.

The three review skills ship in [`.agents/skills/`](.agents/skills/). An
agent that reads project skills finds them there; one that reads only a global
directory runs `scripts/install-skills.sh`. See
[`README.md#skills-for-agent-workflows`](README.md#skills-for-agent-workflows).

The same applies to a PR opened by the user: check the list before pushing, and
report what you skipped.

## Things NOT to do

- Don't unit-test CPAL streams, microphone hardware or permissions. Split the
  logic into pure helpers, test those, and verify the rest manually with
  `cargo run -- <mode>`.
- Don't `.unwrap()` `host.input_devices()` / `output_devices()` /
  `device.default_input_config()`. Use `?` with an `anyhow!` / `Context`
  chain.
- Don't commit `output/mic_test.wav`. `.gitignore` excludes `/output`.
- Don't leave `Todo: …` paths, `unwrap()`-filled stubs, or
  `#[allow(dead_code)]` on functions that should be removed.
- Don't add an async runtime, a new provider, or a new desktop-UI surface
  unless the user asks. See the CPAL section for the version-specific traps.
