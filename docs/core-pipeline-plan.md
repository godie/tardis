# Core pipeline plan: capture → transcribe → process → synthesize → play

Status: **in progress**. PR 1 (`Processor` rename), PR 2 (mock
`Synthesizer`/`Player` loop plus the half-duplex gate) and PR 3 (silence
endpointing and the burst cap) are implemented and merged on `master`;
PR 4 onward is planned and unstarted. Each numbered PR below is independently reviewable, stays
green on `cargo check && cargo test`, and adds a manual path
(`cargo run -- <mode>` or the Tauri shell) so the CPAL/HTTP glue is
verifiable per `DEVELOPMENT_RULES.md`.

Scope: finish the core voice loop (`AGENTS.md` → "Product direction: core
pipeline first") without breaking the two rules the handbook is strictest
about: **no new async runtime** and **no new provider without an explicit
go-ahead**. Section 4 (streaming) and the TTS provider in PR 5 both need
your sign-off before they are built.

---

## 1. Current state map

### 1.1 The five stages

| # | Stage | Status | Where it lives | Notes |
|---|-------|--------|----------------|-------|
| 1 | Capture | **real (mic only)** | `src/audio/capture.rs` (`DefaultInput`, `MicInput`), `src/audio/chunker.rs`, `src/audio/mic.rs`, `src/audio/recorder.rs` | CPAL input at the device's default config (f32 samples, device rate and channel count), fixed-window capture plus silence endpointing (hangover / min-speech / burst cap) in `src/audio/endpoint.rs`, silence classification in `src/audio/activity.rs`, volume in `src/audio/volume.rs`. Saved WAVs are 16-bit (`config::WAV_BITS_PER_SAMPLE`). No system/loopback source. |
| 2 | Transcribe | **real, two contracts** | `src/transcription/transcriber.rs` (`Transcriber`), `src/transcription/local_whisper.rs`, `src/transcription/mock_local.rs`, dispatch in `src/transcription/mod.rs` (`ProviderKind` / `build_provider`) | `mock-local` is a deterministic stub, `local-whisper` is a real HTTP call to the faster-whisper Docker server (`docker/faster-whisper/`). |
| 3 | Process | **mock only** | `src/translation/translator.rs` (`Translator`), `src/translation/mock.rs` (`MockTranslator`), used inline in `src/transcription/live_local.rs` and `src/app/service.rs` | No LLM client exists anywhere in the tree. `reqwest` is already a dependency (blocking + multipart + json). |
| 4 | Synthesize | **does not exist** | — | No TTS module, no speech types, no reference in `src/config.rs`. |
| 5 | Play | **does not exist** | — | `host.output_devices()` is only enumerated for `cargo run -- devices` (`src/audio/devices.rs`). Nothing ever writes audio out. |

The live call path that ships today, in one line:

```
cargo run -- live-local-transcribe  /  Tauri start_live_transcription
  -> transcription::live_local::run_live_local_transcription_with_config_and_events
     -> MicInput (CPAL)              capture
     -> calculate_average_volume     silence gate
     -> audio::endpoint             utterance boundaries (hangover / min-speech / cap)
     -> write_chunk_wav              one temp WAV per utterance (output/live_chunks/)
     -> provider.transcribe(path)    transcribe  (LocalTranscriptionProvider)
     -> MockTranslator               process     (constructed inline, line ~173)
     -> app::live_events::*          AppEvent
     -> format_app_event_for_console  (CLI)  |  app_event_to_ui_event -> emit("app-event")  (Tauri)
```

Facts that constrain the design:

- **The process stage is not injectable.** `live_local.rs` builds
  `MockTranslator::new()` inside the loop. `AppService::run_mock_text_flow`
  does the same. Any real processor has to be injected first; there is no
  seam today.
- **Two transcriber contracts exist.** `Transcriber::transcribe_chunk(&self,
  usize, &[f32]) -> Option<TranscriptionResult>` is used only by the CPAL
  mock pipelines (`transcription/pipeline.rs`, `file_pipeline.rs`). The
  *live* path uses `LocalTranscriptionProvider::transcribe(&self, path:
  &str) -> Result<String>` because the real provider is file-based. Adding
  two more stages on top of two competing contracts would quadruple the
  surface, so PR 1–2 pin the stage shape (see §2.1).
- **Audio never leaves the process boundary in a form a later stage can
  use.** Chunks exist as `Vec<f32>` in the loop, then become a temp WAV,
  then become a `String`. Playback needs f32 + sample rate + channels back.
- **Settings are flat and permissive.** `AppRuntimeConfig`
  (`src/app/config.rs`) is `{ transcription_provider, source_language,
  target_language, chunk_duration_ms, volume_threshold }`, persisted by
  `src/app/settings_store.rs` to `<OS config dir>/tardis/runtime.json` with
  an atomic write-then-rename. `deserialize_ignores_unknown_fields` is a
  locked test, so *adding* a field is backward compatible by construction.
- **`app/` holds no handles.** `src/app/mod.rs` states the layer never
  opens CPAL/Docker/HTTP/FS handles itself. That invariant is what makes
  §2.4's stage traits necessary: the runner injects hands, the `app/` layer
  only decides.

### 1.2 Where the docs no longer match the code

These are real divergences. The **doc text below is already corrected**
in `AGENTS.md` / `ROADMAP.md`; what remains for each PR is the code-side
work, noted per item.

1. **`AGENTS.md` names the wrong transcription abstraction.**
   Handbook: *"Real transcription backends must land behind the existing
   `transcription::transcriber::Transcriber` abstraction."* The shipped real
   backend (`local-whisper`) does **not** implement `Transcriber`; it
   implements `LocalTranscriptionProvider` (file in, text out), because
   `Transcriber` takes `&[f32]` and cannot express an HTTP provider that
   needs a container. `Transcriber` today drives only the mock CPAL
   pipelines. `AGENTS.md` now documents both traits and which one new
   backends implement; PR 1 renames `Translator` → `Processor` in the same
   handbook text (see §4.1 for the choice).
2. **`ROADMAP.md` §1 "Next": *"Wire live `audio` capture chunks into
   `AppService`, so the live path and the mock flows share one
   orchestrator."*** Live capture already ships and deliberately does *not*
   go through `AppService`: it runs in `transcription/live_local.rs` and
   emits through `app::live_events`. `AppService`'s own module doc forbids
   it holding a stream. The "next step" as written cannot be taken;
   `ROADMAP.md` now points at injected stage traits instead (PR 1–3 here).
3. **`ROADMAP.md` §1 "Next": *"Surface provider selection through
   `AppRuntimeConfig` and propagate it from the UI / CLI flag."*** Already
   shipped: `transcription_provider` on the config, `#provider-select` in
   `ui/index.html`, `--provider` in the CLI. Moved from "Next" to "What
   exists"; no further action.
4. **`ROADMAP.md` §3 "Later": *"Add session-level language preferences"*,
   and §4 gap *"Translation is still mock-only."*** The language half is
   shipped (`source_language` / `target_language` flow from the settings
   panel into the event payloads), so §3 now lists only source-language
   detection; the provider-backed half of §4 remains, and that is PR 4 here.
5. **`ROADMAP.md` §4 does not mention that the process stage is hardcoded.**
   It said translation is mock-only, but not that `MockTranslator` is
   constructed *inside* the live loop, which is the actual blocker. §4 now
   states it and points at PR 1 and PR 4.

Verified as still accurate (no action): the CPAL 0.18 gotchas, the module
layout table, and `ROADMAP.md` §2's "utterance-by-utterance via temporary
WAV files, not true streaming". Test counts move with every PR; `AGENTS.md`
and `README.md` carry the current number (305 CLI tests, 28 Tauri tests).

---

## 2. Target design

### 2.1 Stage boundaries

Five stages, five contracts, each synchronous and object-safe, one
deterministic mock each. Stages 4 and 5 are new; stage 3 replaces what
exists; stages 1–2 keep their current types until §4 is approved.

| Stage | Existing contract | Target contract | Mock |
|-------|------------------|-----------------|------|
| Capture | `MicInput::next_chunk` | unchanged | n/a |
| Transcribe | `LocalTranscriptionProvider` | unchanged for now | `mock-local` |
| Process | `Translator` | **`Processor`** | `MockProcessor` |
| Synthesize | — | **`Synthesizer`** | `MockSynthesizer` |
| Play | — | **`Player`** | `MockPlayer` |

Every stage returns data, never events; the runner converts results to
`AppEvent` through `app::live_events` builders, which keeps the
event-construction path (and its tests) in one place.

### 2.2 `Processor` replaces `Translator`

`src/processor/processor.rs` (moved from `src/translation/translator.rs`):

```rust
/// One processed utterance.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub kind: OutputKind,          // Translate | Respond
    pub source_text: String,
    pub text: String,              // the rendered output
    pub source_language: String,
    pub target_language: String,   // output language, both modes
    pub is_final: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputKind { Translate, Respond }

/// Synchronous, object-safe, no IO in the signature.
///
/// `None` is a normal outcome, not an error: empty/whitespace transcript,
/// translate-mode no-op, or a respond-mode decision that says "do not
/// act". The runner skips synthesis entirely on `None`.
pub trait Processor {
    fn process(&self, transcript: &str, mode: &ProcessMode) -> Option<Output>;
}
```

Deliberate choices:

- **`Option<Output>`, not `Result`.** The `Translator` contract already
  uses `Option` for "nothing to do"; a failure that the user must see is a
  `Result` concern that belongs to the *provider impl* (it can log or
  surface through the runner's error event). Keeping the trait
  `Option`-only preserves the existing call-site shape and keeps
  `MockProcessor` a two-line function.
- **`mode` is passed per call, not stored in the impl.** The same
  processor instance serves both modes, so translation and response differ
  only in prompt, output language, and acting rule. A mode switch becomes a
  config change instead of a rebuild.
- **`ProcessMode` carries the language pair per call and is never
  persisted.** As implemented it is a pure runtime argument with no serde
  derives, so nothing about it reaches `runtime.json`. §2.4's `mode` field
  stores only the mode discriminant: `source_language` / `target_language`
  stay the only stored copy of the languages, which is why no migration is
  needed and why the two cannot drift.
- **`OutputKind` travels with the result**, so the event layer never has to
  guess which mode produced a line.

`src/processor/mod.rs` gets the dispatch, mirroring
`transcription::ProviderKind` exactly:

```rust
pub enum ProcessorKind { Mock, Llm }
impl ProcessorKind {
    pub fn from_name(name: &str) -> Option<Self>;
    pub fn name(&self) -> &'static str;   // "mock" | "llm"
    pub fn build(&self) -> Box<dyn Processor>;
}
pub fn build_processor(config: &AppRuntimeConfig) -> Result<Box<dyn Processor>>;
```

`AppService` stops constructing `MockTranslator` inline as part of the same
PR: `new(config)` keeps defaulting to the mock processor, and
`with_processor(config, Box<dyn Processor>)` is the injection point the
mock-flow tests use. The `app/` layer still owns no handle — it holds a
boxed trait object whose impl owns the I/O.

### 2.3 `Synthesizer` and `Player`

`src/synthesis/mod.rs`:

```rust
/// What to say.
#[derive(Debug, Clone, PartialEq)]
pub struct Speech {
    pub text: String,
    pub language: String,
}

/// Rendered audio, stage-agnostic (no CPAL types in the contract).
#[derive(Debug, Clone, PartialEq)]
pub struct AudioChunk {
    pub samples: Vec<f32>,   // interleaved, -1.0..=1.0
    pub sample_rate: u32,
    pub channels: u16,       // 1 for every provider we plan to use
    pub is_final: bool,
}

pub trait Synthesizer {
    fn synthesize(&self, speech: &Speech) -> Option<AudioChunk>;
}

/// Pure helper, shared by the mock and the real player: the duration the UI
/// reports and the runner logs.
pub fn chunk_duration_ms(chunk: &AudioChunk) -> u64;
```

`src/audio/player.rs` (CPAL glue lives under `audio/`, same split as
`audio/capture.rs`):

```rust
pub trait Player {
    fn play_chunk(&mut self, chunk: &AudioChunk) -> anyhow::Result<()>;
    fn stop(&mut self) -> anyhow::Result<()>;
}

pub struct MockPlayer { played: Mutex<Vec<AudioChunk>> }   // records, plays nothing
pub struct CpalPlayer { /* PR 5 */ }
```

`&mut self` on `Player` because a real player owns a stream handle and a
queue; `Synthesizer` stays `&self` like every other stage.

`MockSynthesizer` is deterministic in the strict sense the repo already
enforces: `n = ceil(sample_rate * ms_per_char * chars)`, filled with a
fixed-amplitude square wave at a fixed frequency, no randomness, no clock.
Its tests assert sample count and duration for a fixed string, plus that
empty text returns `None`.

Wiring, keeping the existing public signature (so `src-tauri` does not
change):

```rust
// unchanged public entry point
pub fn run_live_local_transcription_with_config_and_events<F>(config, seconds, should_stop, on_event)
  -> builds provider, processor, synthesizer, player from `config` via the `build_*` dispatchers
  -> run_live_loop_with_stages(&*provider, &*processor, &*synthesizer, &mut *player, ...)

// internal, testable seam: stages injected, no config lookups
fn run_live_loop_with_stages(...)
```

This is the smallest change that makes the loop injectable: the Tauri
command and the CLI keep calling the same function, and the mock Tauri path
still works because the default config yields mock stages.

### 2.4 `mode` on `AppRuntimeConfig` — conflict with the brief, smaller proposal

The brief asks for:

```rust
mode: Translate { source, target } | Respond { language, policy }
```

That duplicates two fields that already exist at the top level and are
already persisted, validated, surfaced in the UI and copied into every
event. Migrating them inside `mode` means: a `runtime.json` migration that
moves data (AGENTS.md wants `legacy_*` tests for it), every
`AppRuntimeConfig` construction site updated, and two ways to express the
same language pair until the old fields are deleted.

**Smaller proposal (recommended):** keep `source_language` /
`target_language` as the single source of truth for languages; make `mode`
a discriminant plus the only thing that is genuinely mode-specific — the
acting policy.

```rust
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ProcessMode {
    #[default]
    Translate,
    Respond { #[serde(default)] policy: RespondPolicy },
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RespondPolicy {
    /// Who the host is. Matched case-insensitively against the transcript.
    pub host_names: Vec<String>,
    /// v1 defaults, and validation rejects anything looser.
    pub answer_questions_only: bool,   // default true
    pub require_name: bool,            // default true
    pub allow_commitments: bool,       // default false, must stay false in v1
}
```

`AppRuntimeConfig` gains one field:

```rust
#[serde(default)]
pub mode: ProcessMode,
```

Why this is the smaller change:

- Old `runtime.json` files have no `mode` key → deserializes as
  `Translate`, which is exactly today's behavior. No data migration, no
  `legacy_*` move test beyond the one that locks this in.
- In respond mode the output language *is* `target_language` (the language
  the host speaks), so `Respond { language }` would be a second name for an
  existing field. `source_language` stays the input language.
- `validate_runtime_config` keeps its fail-fast order and gains one block
  after the language checks:
  - respond mode with empty `host_names` → error naming `mode.policy.host_names`;
  - any host name empty/whitespace after trimming → error;
  - `allow_commitments == true` → `"allow_commitments is not supported yet; respond mode never speaks commitments"` (this is a product promise, not a TODO);
  - `answer_questions_only == false` or `require_name == false` in v1 →
    rejected, so the shipped rule stays the conservative one until we have
    evidence otherwise.
- `normalize_runtime_config` trims + dedupes `host_names`
  (case-insensitive), leaves the rest.

Required `settings_store` tests (AGENTS.md: a new `AppRuntimeConfig` field
lands with `padded_*` / `legacy_*` / `corrupt_*` coverage in the same
commit):

| Test | Asserts |
|------|---------|
| `legacy_json_without_mode_loads_as_translate` | pre-`mode` file → `ProcessMode::Translate` |
| `legacy_json_with_mode_null_loads_as_translate` | `"mode": null` tolerated by `#[serde(default)]` |
| `round_trip_preserves_respond_mode` | host names + policy survive save→load |
| `corrupt_json_with_unknown_mode_kind_is_rejected` | `"kind":"interrogate"` → `SettingsStoreError::Parse`, no silent fallback |
| `save_rejects_respond_mode_without_host_names` | `Validation`, nothing written |
| `save_rejects_allow_commitments_true` | `Validation`, nothing written |
| `padded_host_names_are_normalized` | `"  Ana "`, `"ana"` → single `"Ana"` |

### 2.5 Event surface: `Translation` → `Output`, new `Speech`

`src/app/events.rs`:

```rust
pub enum AppEvent {
    StatusChanged(AppStatus),
    Transcript(AppTranscriptEvent),
    Output(AppOutputEvent),      // was Translation(AppTranslationEvent)
    Speech(AppSpeechEvent),      // new
    Error(AppErrorEvent),
}

pub struct AppOutputEvent {
    pub chunk_index: usize,
    pub kind: OutputKind,        // reused from src/processor
    pub source_text: String,
    pub text: String,            // was translated_text
    pub source_language: String,
    pub target_language: String,
    pub is_final: bool,
}

pub struct AppSpeechEvent {
    pub chunk_index: usize,
    pub text: String,            // what was spoken
    pub language: String,
    pub duration_ms: u64,
    pub is_final: bool,
}
```

Knock-on changes, all in the same PR as the rename (PR 1 for `Output`)
except `Speech`, which lands with the mock loop (PR 2):

- `app_event_kind` → `"output"` / `"speech"`; `format_app_event_for_console`
  → `[chunk 3][output translate en->es] hola`, `[chunk 3][speech es 1240ms] hola`.
- `src/app/live_events.rs` → `build_output_event(...)`,
  `build_speech_event(...)` (pure, tests for each field).
- `src/app/ui_events.rs` → `UiAppEvent.kind` gains `"output"` and
  `"speech"`; two new optional fields `output_kind: Option<String>` and
  `duration_ms: Option<u64>`; `kind: "speech"` populates `text`,
  `target_language` (the spoken language) and `duration_ms`.
- `ui/app.js` → `case "translation"` becomes `case "output"` (same panel,
  same `appendTranslation` call), new `case "speech"` writes
  `Spoken · 1.2 s` into a status line under the translation panel;
  `updateLanguageHint` copy loses "mock translator".
- `ui/index.html` → one new `<p id="speech-status" class="speech-status" role="status" aria-live="polite">`,
  styled from existing tokens (`--ink-soft`, `--signal-surface` for the
  active state). No new control surface.

The UI is a display change only: no new panels, no player controls.

### 2.6 The LLM client

**Location.** `src/processor/llm.rs`, next to its trait, mirroring
`src/transcription/local_whisper.rs` (HTTP client + pure helpers + tests).
Not in `app/` (which may not hold a client) and not in `src-tauri` (CLI must
be able to run it too).

**Transport.** `reqwest::blocking::Client` — already a dependency and
already used for faster-whisper. **No async runtime**, per AGENTS.md. A
20 s timeout, one request in flight, `OpenAI`-compatible
`POST {base}/chat/completions` so the same code drives Ollama
(`http://localhost:11434/v1`), llama.cpp server, LM Studio or a cloud
endpoint by changing a base URL.

**Configuration.** Constants in `src/config.rs`, alongside
`LOCAL_WHISPER_*`:

```rust
pub const LLM_BASE_URL: &str = "http://localhost:11434/v1";
pub const LLM_MODEL: &str = "qwen2.5:7b-instruct";
pub const LLM_TIMEOUT_SECS: u64 = 20;
pub const LLM_MAX_TOKENS: u32 = 256;
pub const LLM_TEMPERATURE: f32 = 0.2;
pub const PROCESS_PROMPT_VERSION: u32 = 1;
```

`AppRuntimeConfig` gains `processor_provider: String` (`"mock" | "llm"`,
validated like `transcription_provider`, with a lock-in test on the
supported list) and optionally `llm_model: String`. **It never gains a
secret.**

**API key handling.**

- Read once at construction from `TARDIS_LLM_API_KEY`; empty is valid (local
  Ollama needs none).
- Held in a `Secret(String)` newtype whose `Debug` and `Display` print
  `"***"`, so no `{:?}` on a config, log line or error can leak it.
- Never serialized: not a field of `AppRuntimeConfig`, never written by
  `settings_store`, never in `runtime.json`, never in an `AppEvent` /
  `UiAppEvent` payload.
- Errors are mapped through `redact_secret(&str, &Secret) -> String` (pure,
  tested) before they reach `AppEvent::Error`, because HTTP libraries echo
  request headers in some messages.
- OS keychain integration is explicitly **out of scope**; `AGENTS.md`'s
  "no new surface without a go-ahead" covers it.

**Mocking in tests.** Three layers, none of which touch the network:

1. `MockProcessor` for pipeline/event tests (deterministic strings, same
   shape as `MockTranslator`'s output).
2. Pure helpers in `llm.rs` for everything that can break:
   `build_chat_request_body(model, prompt, max_tokens)` →
   `parse_chat_completion(json) -> Result<String>`,
   `map_http_status(u16) -> LlmError`, `redact_secret(...)`,
   `build_translate_prompt(...)` / `build_respond_prompt(...)`. Same split
   as `local_whisper.rs`, which has 8 tests and zero network calls.
3. A `--processor mock` CLI path in every mode, so CI and a laptop without
   Ollama still exercise the whole loop.

**Prompt hygiene.** Transcripts are untrusted input: they are wrapped in
`<transcript>…</transcript>` inside a system prompt that says the content
is data, never instructions, and the model's reply is only displayed and
spoken — never parsed as a command. The prompt is versioned by
`PROCESS_PROMPT_VERSION` so a prompt change is visible in logs and in
review.

---

## 3. Incremental PR plan

Each PR: `cargo check && cargo test` plus
`cargo test --manifest-path src-tauri/Cargo.toml` when `src/app/` or
`src-tauri/` is touched, the manual command listed, and the matching
`DEVELOPMENT_RULES.md` rows in the same commit. Conventional Commits, no
AI attribution trailers.

### PR 1 — `Translator` → `Processor` (rename, no behavior change)

**Why first (reordered after your review).** It gives the loop its final
shape *before* anything is built on it: `Processor::process(&self, text,
&ProcessMode) -> Option<Output>`, `OutputKind`, `ProcessorKind` /
`build_processor`, and `AppEvent::Output`. The mock voice loop then lands on
the real seam instead of being migrated twice.

**Files.** `src/translation/` → `src/processor/` (`mod.rs`,
`translator.rs` → `processor.rs`, `mock.rs` keeping `MockProcessor`'s output
byte-identical to `MockTranslator`'s, `pipeline.rs`), `src/lib.rs`,
`src/main.rs` (`mock-translate` keeps its mode name), `src/app/service.rs`,
`src/app/state.rs` (`apply_output`), `src/app/events.rs` (`Output`),
`src/app/live_events.rs` (`build_output_event`), `src/app/ui_events.rs`,
`src/transcription/live_local.rs`, `ui/app.js` (`case "output"`), docs.

**Tests.** The 10 `translation::mock` tests move to `processor::mock`
unchanged; `Output.kind == OutputKind::Translate` in translate mode;
`AppEvent::Output` payload tests; `ui_events` JSON key renames;
`app_event_kind` → `"output"`; `AppService` mock-flow tests assert `Output`
instead of `Translation`; `ProcessorKind::from_name` / `name` /
`build_processor` dispatch (default `mock`).

**Docs.** `AGENTS.md` layout + the `Translator` paragraph; the
`DEVELOPMENT_RULES.md` rows renamed; `ROADMAP.md` §3 wording.

**Manual.** `cargo run -- mock-translate` prints exactly what it printed
before this PR (diff the output); `cargo run -- app-mock-flow` unchanged;
Tauri shell fills the output panel.

**Risks.** Wide but shallow: 20+ files, all compile-time. The one behavioural
trap is the mock string; keep `"[mock {target}] mock translation:
\"{text}\""` verbatim so the only output change in the whole plan is PR 5's
provider swap.

**`ProcessMode` is a runtime argument, never stored state.** It carries the
language pair per call (`Translate { source_language, target_language }`)
and derives no serde traits, so it cannot reach `runtime.json`.
`AppRuntimeConfig.source_language` / `target_language` remain the only stored
copy of the languages, and PR 6's `mode` field adds nothing to them: it
stores a discriminant and reads the pair back from those two fields.
Persisting the languages in both places would let the two copies drift.

### PR 2 — `Synthesizer` + `Player` mocks and the half-duplex gate; the loop runs end to end on mocks

**Status.** Implemented on the branch stacked on top of PR 1, so it is
written directly against `Processor` / `Output`.

**Why here.** It proves the five-stage shape and the `Speech` event while
the only real stages are the two we already trust, and it adds no provider,
so it needs no approval.

**Files.** `src/synthesis/mod.rs` (`Synthesizer`, `Speech`, `AudioChunk`,
`chunk_duration_ms`), `src/synthesis/mock.rs` (`MockSynthesizer`),
`src/audio/player.rs` (`Player`, `MockPlayer`), `src/audio/mod.rs`
(re-export), `src/audio/gate.rs` (`HalfDuplexGate`, pure),
`src/app/events.rs` (`AppEvent::Speech`, `AppSpeechEvent`, kind label,
console format), `src/app/live_events.rs` (`build_speech_event`),
`src/app/ui_events.rs` (`kind: "speech"`, `duration_ms`),
`src/transcription/live_local.rs` (mock synth + mock player after the
output stage; consult the gate before draining),
`src/main.rs` (`mock-voice-flow` mode + doc header), `ui/app.js`,
`ui/index.html`, `ui/styles.css` (speech status line), plus the two docs.

**Tests.** `chunk_duration_ms` for mono/44.1k/16k, stereo, 0 samples and
zero sample-rate/channel guards; `MockSynthesizer` determinism (same text →
same length twice), empty text → `None`, characters-not-bytes, full-scale
bound; `MockPlayer` records exactly one chunk per `play_chunk`, sums
durations, `stop` is idempotent; `build_speech_event` field mapping;
`app_event_kind`/console format for both new variants; `ui_events` speech
mapping and JSON shape (no `translated_text` key on a speech payload);
**`HalfDuplexGate` table** — idle allows draining, playing blocks it, the
block survives until playback finishes, and the gate cannot be entered
twice (see below).

**Development rules rows.** `synthesis::mock`, `audio::player`,
`app::live_events` (extend existing row).

**Manual.** `cargo run -- mock-voice-flow` → prints
`[chunk 1][speech es 1240ms] [mock es] mock translation: "…"` with no mic
and no Docker. Then `cargo run -- live-local-transcribe` → same speech
lines after each transcript, with `played` counts from `MockPlayer`.

**Half-duplex gate (mandatory here).** Pausing the drain is **not** enough:
the capture callback keeps filling the ring buffer while the player talks,
so the first drain after playback would transcribe our own TTS. While
speaking, captured samples are **discarded**, the buffer is **cleared on
`end_speaking`**, and a **reverb tail** (default **250 ms**) keeps
discarding afterwards to cover room reflections and device latency. Shape:

```rust
// src/audio/gate.rs — pure, no CPAL, no clock of its own
pub const DEFAULT_TAIL_MS: u64 = 250;

pub struct HalfDuplexGate { speaking: bool, tail_ms: u64, tail_remaining_ms: u64 }
impl HalfDuplexGate {
    pub fn new(tail_ms: u64) -> Self;
    pub fn with_default_tail() -> Self;
    pub fn begin_speaking(&mut self) -> bool;  // false if already speaking
    pub fn end_speaking(&mut self);            // speaking = false, tail = tail_ms
    pub fn is_speaking(&self) -> bool;
    pub fn tail_remaining_ms(&self) -> u64;
    pub fn drain_allowed(&self) -> bool;       // !speaking && tail == 0
    pub fn advance(&mut self, captured_ms: u64); // counts the tail down
}
```

The runner owns one gate and calls it in this order:

```text
begin_speaking() -> play_chunk() (blocking) -> end_speaking()
  -> drain the ring buffer and throw it away, advancing the tail
  -> normal draining resumes only when drain_allowed()
```

The gate owns no clock: the caller advances it by the milliseconds it
captured or dropped, which is what keeps the decision unit-testable. The
mandatory regression test is the sequence `begin_speaking` → capture →
`end_speaking` → drain yields nothing, and capture after the tail drains
normally.

**Risks.** Two more stages in a serial loop make each iteration slower on
mocks too, but by microseconds. The risk is event volume, one `speech`
line per chunk. Emit speech only when synthesis returns `Some`; a `None`
output produces no event. The gate adds a stall of up
to one utterance; PR 3 replaces clock chunks with endpointing so the stall
is a pause between utterances instead of dropped audio.

### PR 3 — Silence endpointing and the burst cap (whole utterances, not clock chunks)

**Status.** Implemented and merged: `src/audio/endpoint.rs` (pure, 20 tests),
the three `AppRuntimeConfig` fields (serde-defaulted so an older
`runtime.json` still loads), the utterance-based live loop, the settings
panel fields and `cargo run -- utterance-test`.

**Why (added after your review).** The LLM must receive a whole utterance.
Today the pipeline cuts on `chunk_duration_ms` and sends one transcript per
clock chunk, which either splits a sentence in half or concatenates two
speakers. Endpointing at the capture layer fixes the input contract for
every stage above it, including respond mode's decision rule.

**Design.** Reuse [`crate::audio::activity::is_speech_like`] rather than
introducing a second VAD. The existing classifier is `volume > threshold`,
which is the signal endpointing needs. Add a hangover window so a pause
mid-sentence does not close the utterance, and a **burst cap** so one long
run is not buffered forever.

```rust
// src/audio/endpoint.rs — pure, frame-counted, no wall clock
pub struct EndpointConfig {
    pub chunk_duration_ms: u64,  // from AppRuntimeConfig, unchanged
    pub hangover_ms: u64,        // default 400: frames to tolerate as a pause
    pub min_speech_ms: u64,      // default 200: shorter bursts are noise
    pub max_utterance_ms: u64,   // default 15_000: burst cap -> force flush
}

pub enum EndpointDecision { Continue, Utterance { frames: usize }, Discard }

pub fn advance(state: &mut EndpointState, frame_is_speech_like: bool, cfg: &EndpointConfig)
    -> EndpointDecision;
```

Rules (all table-tested): speech starts an utterance; `hangover_ms` of
consecutive non-speech frames closes it, provided it lasted ≥
`min_speech_ms`; a burst longer than `max_utterance_ms` flushes at the cap
and starts a new utterance immediately; a burst shorter than
`min_speech_ms` is `Discard`ed (no event, no WAV, no provider call).
Samples are accumulated into one buffer and written **once** per utterance,
so the per-chunk WAV write and the lost-buffer bursts from the `while let`
drain disappear.

**Files.** `src/audio/endpoint.rs` (new, pure + tests), `src/audio/mod.rs`,
`src/app/config.rs` (`hangover_ms`, `min_speech_ms`, `max_utterance_ms` +
validation + legacy/`padded_*`/`corrupt_*`/`normalize` tests — three new
fields in one commit, per AGENTS.md), `src/app/settings_store.rs`,
`src/config.rs` (defaults), `src/transcription/live_local.rs` (loop drains
frames through the endpoint state instead of cutting on the clock),
`ui/index.html` + `ui/app.js` (three numeric fields, same shell styling),
`src/main.rs` (`utterance-test`, no mic: feed a synthetic frame pattern
through the pure state machine and print the decisions), docs.

**Tests.** `advance` table: single burst + hangover → one `Utterance`;
short blip → `Discard`; pause shorter than hangover → `Continue`; pause
longer → `Utterance`; exactly at `min_speech_ms` → accepted (boundary);
burst at `max_utterance_ms` → forced `Utterance` at the cap and a fresh
utterance after; silence-only stream → no decision ever becomes an
`Utterance`. Config validation for the three fields, plus the legacy and
`padded_*` tests. Pure helper test for `frames_to_ms` (the inverse of
`chunk_duration_ms`), since the UI reports utterance length.

**DEVELOPMENT_RULES rows.** `audio::endpoint` (pure), the runner row
updated (utterance-based, still manual), `app::config` extended.

**Manual.** `cargo run -- utterance-test` prints the decision sequence for
a synthetic pattern; `cargo run -- live-local-transcribe` now prints one
transcript line per utterance — speak a full sentence with a pause in the
middle and confirm it stays one line; stay silent for 30 s and confirm no
provider call happens.

**Risks.** (a) `hangover_ms` too short splits sentences, too long merges
turns — defaults are conservative (400 ms) and the fields are exposed so you
can tune without a rebuild; (b) the burst cap can cut a genuinely long
monologue mid-word — accepted, because the alternative is unbounded memory;
(c) latency: waiting for the hangover adds `hangover_ms` to every utterance
by construction, which §4's streaming design has to remove.

### PR 4 — Real LLM `Processor`, translate mode

**Files.** `src/processor/llm.rs`, `src/processor/prompt.rs`,
`src/processor/mod.rs` (`ProcessorKind`, `build_processor`),
`src/config.rs` (`LLM_*`), `src/app/config.rs` (`processor_provider`,
`llm_model`, validation + lock-in test), `src/app/settings_store.rs`
(`legacy_json_without_processor_provider_loads_as_mock`),
`src/transcription/live_local.rs` (build processor from config),
`ui/index.html` + `ui/app.js` (processor select, reusing the provider
field styling), `src/main.rs` (`llm-translate-text <text>` for a single
round-trip without a mic), docs (`README` quickstart for Ollama,
`DEVELOPMENT_RULES` rows).

**Tests.** `build_translate_prompt` (languages interpolated, system prompt
contains the untrusted-data sentence, version const referenced);
`parse_chat_completion` (valid, missing `choices`, non-string content);
`map_http_status` (401/429/500/404); `redact_secret` (key in message is
removed, empty key leaves message intact); `processor_provider` validation
and legacy default; **no network test**.

**Manual.** Two backends, one interface (resolved: item 1 in §5). Local
first: `ollama serve && ollama pull qwen2.5:7b-instruct`, then
`cargo run -- llm-translate-text "the meeting moved to Thursday"` → real
Spanish. Then the same against a **fast cloud model** (OpenAI-compatible
`base_url` + `TARDIS_LLM_API_KEY`) to validate the latency claim before
committing to a default. Finally `cargo run -- live-local-transcribe
--processor llm`, which by now feeds whole utterances (PR 3).

**Risks.** (a) latency — a 7B model on CPU can take 1–3 s per *utterance*
(not per second of audio, thanks to PR 3), see §4; (b) non-determinism —
never assert on model output in `cargo test`; (c) prompt injection from
transcripts — mitigated in §2.6; (d) key leakage in error paths —
`redact_secret` on every message before it becomes an event; (e) two
backends means two latency profiles — the config default stays local, and
the cloud path is selected by base URL, not by a separate provider kind.

### PR 5 — Real TTS provider: Piper (local HTTP service in Docker)

**Approved in principle**; ask again only if the shape below grows beyond
one provider. Piper is the core synthesizer (resolved: item 6 in §5);
KittenTTS 2 is a later `Synthesizer` provider behind the same trait, not a
second path. Shape, identical to faster-whisper: `docker/piper/` with
`docker-compose.yml` binding
`127.0.0.1:5002`, a `README.md` in the same five-section shape as
`docker/faster-whisper/README.md`, and `src/synthesis/local_http.rs`
returning raw 22.05 kHz mono PCM that `parse_pcm_body` + `resample_linear`
(pure, tested) turn into an `AudioChunk`. `CpalPlayer` lands in
`src/audio/player.rs`.

**Files.** `docker/piper/{docker-compose.yml,README.md}`,
`src/synthesis/local_http.rs`, `src/synthesis/mod.rs` (dispatch),
`src/audio/player.rs` (`CpalPlayer`, `select_output_device` pure helper),
`src/config.rs` (`TTS_BASE_URL`, `TTS_VOICE`, `TTS_SAMPLE_RATE`),
`src/app/config.rs` (`synthesizer_provider`, `player_device`),
`src/transcription/live_local.rs`, `src/main.rs` (`say-text <text>`, and
`devices` learns `--output`), docs.

**Tests.** `parse_pcm_body` (valid, empty, odd byte count),
`resample_linear` (identity, 22.05k→48k length, single sample), duration
helper at 22.05k, `select_output_device` name matching, config validation
of the two new fields + legacy defaults. `CpalPlayer` itself: no unit tests
(rule 2), manual only.

**Manual.** `docker compose -f docker/piper/docker-compose.yml up -d`,
`cargo run -- say-text "hello from tardis"` → hear it once and exit;
`cargo run -- live-local-transcribe --processor llm --synthesizer piper`.

**Risks.** (a) headless/CI has no output device — `CpalPlayer` must fail
with a clear error, not panic, and the mock path must stay default;
(b) playback can feed back into the mic — the half-duplex gate from PR 2
already **discards** captured audio while playing plus a **250 ms tail**
after playback, and the virtual device in PR 7 completes the isolation;
(c) format mismatch
(22.05k mono → 48k stereo device) — the resampler is the only place that
can silently produce chipmunk audio, so it gets the tests.

### PR 6 — Respond mode with two-layer commitment safety

**Files.** `src/app/config.rs` (`ProcessMode`, `RespondPolicy`,
validation, normalize), `src/app/settings_store.rs` (the seven tests in
§2.4), `src/processor/decision.rs` (layer a: classify + decide),
`src/processor/guard.rs` (layer b: validate a generated reply),
`src/processor/processor.rs` (`Output` gains `blocked: Option<BlockReason>`;
`is_final` keeps its existing meaning), `src/processor/prompt.rs`
(respond prompt), `src/processor/mock.rs` (`OutputKind::Respond`),
`src/processor/llm.rs` (mode routing + guard call),
`src/app/events.rs` (`OutputKind` in the payload), `ui_events.rs`,
`ui/index.html` (mode select + host names input, disabled while a session
runs), `ui/app.js`, `src/main.rs` (`live-local-transcribe --mode respond
--host <name> --dry-run`), docs.

**Layer a — classify the incoming utterance, then decide.** Pure and
table-tested. Question detection accepts a **leading `¿`** as well as a
trailing `?`, because the Spanish source script opens questions with it and
speech recognisers frequently drop the closing punctuation:

```rust
pub fn is_question(text: &str) -> bool;   // trailing '?' OR leading '¿' OR interrogative opener
pub fn is_commitment_request(text: &str) -> bool;  // dates, money, "can you … by …" (acceptance is layer b)
pub fn decide_should_respond(text: &str, policy: &RespondPolicy) -> Decision;
```

`Decision` is `Answer`, `Suggest { reason }`, or `Ignore`. Rules: host name
required (case-insensitive, word boundary, optional trailing punctuation);
a question is required; **a commitment request never auto-speaks** — it
yields `Suggest`, surfaced in the UI as a suggestion line and never
synthesized. Layer a classifies the **incoming request** by category:
dates/booking (`mañana`, `el lunes`, `next week`, `by friday`),
money/amounts (`€`/`$`/`%`/digits + currency words, `cost`, `price`,
`pagar`), and `can you … by …` patterns in both languages the app ships.

**Acceptance words are not layer a's job.** `claro`, `deal`, `lo haré`,
`I'll` in the *incoming* question are just speech; they matter only when
they appear in the **generated reply**, which is exactly what layer b
guards. Keeping them out of the input classifier is what stops a plain
"¿claro que sí?" from being demoted to a suggestion for no reason.

**Layer b — validate the generated reply before synthesis.** Even when
layer a says `Answer`, the LLM can volunteer a promise. `guard` inspects the
reply and blocks it if it contains a date, an acceptance, or an amount:

```rust
pub struct Guarded { pub allowed: bool, pub reasons: Vec<BlockReason> }
pub fn validate_reply(reply: &str) -> Guarded;
```

A blocked reply still emits an `Output` event (so the operator sees what the
model would have said) and carries `blocked: Some(BlockReason)`; the runner
does **not** synthesize or play it. `is_final` keeps exactly the meaning it
has for every other output ("this is the terminal chunk of the utterance")
and is **not** reused as a safety flag — one field, one meaning. This keeps
the failure mode "silent in the room, visible on screen" instead of "the
assistant promised something on stage".

**Tests.** Both tables:

- `decide_should_respond`: name + `?` → `Answer`; name + leading `¿` →
  `Answer`; name only → `Ignore`; question only → `Ignore`; other name →
  `Ignore`; empty → `Ignore`; commitment request with name + `?` →
  `Suggest` (never `Answer`); CJK/emoji transcript → `Ignore`.
- `validate_reply`: plain answer → allowed; any ISO-ish date, weekday,
  `tomorrow`, `mañana` → blocked; acceptance words (`I'll`, `lo haré`,
  `claro`, `deal`) → blocked **here, not in layer a**; `€`, `$`, digits
  with a currency word → blocked; empty/whitespace → blocked (nothing to
  say); a reply that merely *mentions* the amount in the question (quoted
  context) → blocked too, by design: no exceptions in v1.

`is_final` is never touched by the guard: a blocked reply keeps whatever
finality the processor produced, and the block is reported only through
`blocked`.

Plus `ProcessMode` serde round-trip, the seven `settings_store` tests from
§2.4, `normalize` of host names, and a `ui_events` test that
`output_kind == "respond"` and that a blocked reply carries the reason.

**Manual.** `cargo run -- live-local-transcribe --mode respond --host "Ana" --dry-run`,
then the same with a commitment question (`"Ana, can you confirm the price
by Friday?"` → must print a suggestion, never a spoken answer), then
without `--dry-run` to hear a plain answer. Tauri: pick Respond, type a
name, press Start, ask a question out loud.

**Risks.** The worst product risk in this plan is the assistant speaking
when it should have stayed quiet, in front of the host. Mitigations: two
independent layers (input classification and reply validation), `dry-run`
in the CLI, name *and* question required, `allow_commitments` rejected by
validation in v1, and no respond mode in the settings panel until you have
run the CLI path for a session. Residual risk: both layers are lexical, so
an unusual phrasing can slip through — hence the "suggestion only" default
for anything that even looks like a commitment.

### PR 7 — System-audio capture and virtual output device

**Approved in principle** (new capture source; AGENTS.md's "extend, don't
start parallel paths" is satisfied by extending `audio/capture.rs`). Ask
again only if a second capture path becomes necessary.

**Files.** `src/audio/capture.rs` (a `SystemInput` next to `MicInput`
behind `cfg(target_os)`), `src/config.rs`, `src/app/config.rs`
(`audio_source: Mic | System` + validation + legacy test), `src/main.rs`
(`devices --source system`, `live-local-transcribe --source system`),
`docs/system-audio.md` (per-OS setup: BlackHole on macOS, WASAPI loopback
on Windows, PulseAudio monitor on Linux), `README` table row.

**Tests.** Only the pure parts: device-name matching, source-name parsing,
config validation + legacy default. CPAL loopback itself is manual (rule 2).

**Manual.** `cargo run -- devices --source system` lists the loopback
device; `cargo run -- live-local-transcribe --source system
--synthesizer piper` transcribes a video playing on the host.

**Risks.** (a) three platform backends, only one testable here — ship
macOS first, feature-gate the rest with a clear "not supported on this
platform" error, not a `todo!()`; (b) permissions (macOS screen/audio
capture consent, Windows privacy settings); (c) echo — the virtual output
device must be distinct from the virtual input this PR adds, or the
pipeline will transcribe its own speech.

---

## 4. Latency plan

### 4.1 Where the current design breaks

Measured/structural facts, not estimates: the loop sleeps
`chunk_duration_ms` (default 1000, min 250), then drains **every** complete
chunk it finds, and processes each one serially inside the same thread that
owns the microphone. Per chunk it writes a WAV to disk, does one HTTP
round-trip, one more in PR 4, one more in PR 5, and then blocks on
playback.

With the numbers we already have:

| Step | Today | Cost per chunk |
|------|-------|----------------|
| Chunk window | fixed window, no endpointing | 250–5000 ms of *content*, plus up to one window of *wait* |
| WAV write + delete | per chunk | sub-ms |
| faster-whisper on CPU | documented **2–5× realtime** in `docker/faster-whisper/README.md` | 1 s of audio = 2–5 s wall clock on `base` |
| LLM (PR 4) | one blocking call | 0.3–3 s |
| TTS (PR 5) | one blocking call | 0.2–1 s |
| Playback | blocking in the same thread (PR 5) | = duration of the speech |

So the first end-to-end utterance lands **3–10 s** after the person stopped
talking on CPU, and because `next_chunk` keeps filling while we wait, the
loop then catches up in a burst of N chunks (the `while let` drain), each
taking another full round-trip. The transcript therefore arrives in bursts
that fall further behind, and playback runs concurrently with capture,
which both delays the next chunk and risks the mic hearing the speakers.

Four concrete causes:

1. **One thread for five stages.** Capture cannot run while anything blocks.
2. **File-based provider contract.** `LocalTranscriptionProvider::transcribe(&str)`
   forces a WAV encode/decode round-trip per chunk that a streaming decoder
   does not need. (This is the §1.2 item 1 divergence: `Transcriber` takes
   samples; the live path does not use it.)
3. ~~**No endpointing.** Chunks are cut by the clock, not by silence, so a
   sentence is routinely split across chunks (bad for an LLM) and a pause
   wastes a full window.~~ **Resolved by PR 3**: utterances end on silence
   with a hangover window, and a burst cap bounds a long monologue. That
   removes the "sentence split across chunks" failure and stops the
   catch-up burst from being N independent LLM calls; it does **not** remove
   the serial stage cost below, which is what §4.2 addresses.
4. **No stage overlap.** Nothing starts processing chunk N until chunk N-1
   has been fully played.

### 4.2 Proposed streaming interface — needs your explicit approval

Nothing here is built until you say so; the constraint "no new async
runtime" is respected in the proposal itself.

```rust
// PR-N (after PR 5). Still synchronous, still object-safe.
pub trait AudioSource { fn next_frame(&mut self) -> Option<AudioFrame>; }      // wraps MicInput / SystemInput

pub trait StreamingTranscriber {                                              // replaces the file contract
    fn push(&mut self, frame: &AudioFrame);
    fn poll(&mut self) -> Option<TranscriptPartial>;                          // partials allowed now
    fn finish_utterance(&mut self) -> Option<TranscriptionResult>;            // on VAD endpoint
}

pub trait Gate { fn is_endpoint(&self, frame: &AudioFrame) -> bool; }         // pure, testable VAD

pub trait StreamingSynthesizer { fn push(&mut self, speech: &Speech) -> Option<AudioChunk>; }

pub trait PlayerQueue { fn enqueue(&mut self, chunk: AudioChunk) -> anyhow::Result<()>; }
```

Shape: **thread-per-stage with bounded `std::sync::mpsc` channels**, the
same `Arc<AtomicBool>` stop flag and RAII cleanup the Tauri session already
uses. Capture → VAD/endpoint → transcribe → process → synthesize → play,
each stage a dedicated OS thread; `reqwest::blocking` inside the stage
thread is fine because the thread is already dedicated. Bounded queues with
drop-oldest on the capture side guarantee the pipeline can never lag more
than N windows; playback gets half-duplex gating (capture is **discarded**
while speaking, plus a 250 ms tail) until PR 7's virtual device exists.

**Explicitly not proposed without approval:** `tokio`, `async-trait`,
`futures`, background executors. If we ever need N concurrent LLM calls
(e.g. speculative translate + respond in parallel), that is the moment to
ask — one sentence in the request is enough, and it should be a separate
PR that adds the runtime and nothing else.

Cheap wins that need no approval and buy most of the latency back, in order:

1. ~~**Endpointing on silence** (reuse `audio::activity::is_speech_like`
   with a hangover window) — cuts sentences at pauses instead of on the
   clock, which also improves LLM quality more than any model upgrade.~~
   **Resolved by PR 3.**
2. ~~**Cap the burst** — process at most one chunk per tick, drain the rest
   away, so a slow provider cannot build an unbounded backlog.~~
   **Resolved by PR 3**: utterances end on silence and `max_utterance_ms`
   bounds a monologue, so there is no per-chunk backlog left to cap.
3. **Local model warmth** — keep the faster-whisper container and Ollama
   resident (already the case for Docker; document it).
4. **TTS over streaming PCM** — start playback on the first chunk instead
   of waiting for the whole file (still sync, still one thread).

---

## 5. Decisions (was: open questions)

Resolved on review, so the PRs below do not re-litigate them.

1. **Where do LLM calls go — local or cloud?** Keep the OpenAI-compatible
   interface as the *only* interface, then validate it against **both** a
   local Ollama model and a **fast cloud model** before choosing the
   default. Until that measurement exists the shipped default stays local
   (`http://localhost:11434/v1`); a cloud endpoint is a base-URL (+key)
   change, not a code change.
2. **API key storage.** `TARDIS_LLM_API_KEY`, redacted in every log/error,
   never in `runtime.json`. Keychain later, as its own PR.
3. **Does `mode` carry the languages, or stay a tag?** Tag only (§2.4);
   languages stay on `source_language` / `target_language`, so no data
   migration and no duplicated source of truth.
4. **Who is "the host", and how literal is the name rule?**
   `policy.host_names` is a list, matched case-insensitively with word
   boundaries and optional trailing punctuation; two hosts per session in
   v1.
5. **Can respond mode ever speak a commitment?** No. Two independent
   layers (PR 6): classify the incoming request (**dates, money, and
   "can you … by …"** → suggestion only, never spoken) and validate the
   generated reply before synthesis (date / acceptance / amount → shown in
   the UI but not synthesized). Acceptance words belong to the reply guard,
   not to the input classifier. Question detection accepts a **leading
   `¿`** as well as a trailing `?`, because recognisers drop the closing
   mark.
6. **Which TTS provider?** **Piper** for core (PR 5), over a local HTTP
   Docker service — CPU-fast, permissive licence, small models, and the
   exact pattern faster-whisper already proves. **KittenTTS 2** is a later
   `Synthesizer` provider behind the same trait, not a parallel path.
7. **Which output device?** System default output in PR 5, so the loop is
   audible immediately; virtual input device in PR 7.
8. **When does the `Speech` event fire — synthesis or playback?** After
   synthesis succeeds, before playback starts: the UI shows intent, and
   playback failures surface as a separate `Error`. The payload carries
   `duration_ms`, not "played" state.
9. **Does the processor see one utterance or the session?** The utterance
   plus a bounded ring buffer of the **last 6 utterances** inside the
   runner — measured in utterances, never in seconds, and filled only by
   endpointed utterances (PR 3), not by clock chunks. No persistence, no
   export (session export is still the `ROADMAP.md` §4 gap and stays out of
   this plan).
10. **Do we need a `dry_run` in the Tauri UI too?** CLI only in PR 6; add
    the UI toggle after you have run respond mode by hand for a session.

**Still needs your explicit go-ahead:** any async runtime (§4). PR 5
(Piper) and PR 7 (system audio) are approved in principle; PRs 1–4, 6 need
no further approval.
