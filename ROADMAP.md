# TARDIS Roadmap

> **TARDIS** = **T**ranscription **A**nd **R**eal-time **D**ual-input **I**nterpretation **S**ystem.

Each axis is a separable workstream. Per axis below: **Status** (verified against current `master` and the test counts in `cargo test`), **Gap** (what's still missing), and **Next** — the smallest genuine improvement, split into **pure helper(s) + unit tests** and **a matching manual CLI mode**, per [`DEVELOPMENT_RULES.md`](./DEVELOPMENT_RULES.md).

Status legend: ✅ shipped · ⚠️ partial · ⏳ not yet started.

## T — Transcription

- **Status:** ✅ mock shipped; real API slot is open.
- **There:** `src/transcription/mock.rs` (pure `mock_transcribe_chunk` + 8 unit tests), `src/transcription/pipeline.rs` (CPAL driver), `cargo run -- mock-transcribe`.
- **Gap:** no real transcription API yet. `mock_transcribe_chunk` is the swap point.
- **Next:**
  - **Pure:** introduce `pub trait Transcriber { fn transcribe(&self, chunk: &[f32]) -> Option<String> }` in `src/transcription/mod.rs`. Ship a real impl behind a feature flag (`whisper-local` or `remote-http`).
  - **Unit tests:** 3–4 tests against a fake `Transcriber` (echoes chunk, refuses empty chunk, normalises whitespace, idempotent re-run).
  - **Manual CLI:** `cargo run -- real-transcribe` — same drain loop as `mock-transcribe`, prints `transcript: <text>` per chunk.
  - **Switch:** `mock_transcribe_chunk` becomes `MockTranscriber`; `run_mock_transcription_test` flips to a `#[cfg(feature = "mock")]` arm.

## A — And

Conjunction in the name; nothing architectural to plan. Tracked for completeness so the five axes stay balanced.

## R — Real-time

- **Status:** ⚠️ chunked live capture shipped; drain loop is timer-based, not reactive.
- **There:** `src/audio/chunker.rs` (pure chunking helpers + 11 unit tests), `src/audio/mic.rs` (live mic loop), `cargo run -- chunk-test` (10 s × 1 s chunks), `cargo run -- mic` (live until Ctrl+C).
- **Gap:** drain sleeps `chunk_duration_ms` and then drains every complete chunk in one pass — fine for v0.1.0 but falls behind on scheduling jitter. Tightening is a reactive drain via `condvar`/`mpsc`.
- **Next:**
  - **Pure:** `pub fn percentiles(samples_ms: &[u128]) -> Option<(u128, u128, u128)>` in a small `src/audio/stats.rs` — `None` for empty input, `Some((P50, P95, P99))` otherwise; nearest-rank selection. + 5 tests covering empty input (`None`), single sample (`Some((x, x, x))`), constant input, sorted `1..=100` vs unsorted equal-magnitude input, and a 10 000-element sanity check. The chunking math already has helpers; this is the only new pure piece on this axis.
  - **Manual CLI:** `cargo run -- realtime-test` runs 10 s, measures ms between chunk-arrival and chunk-print, hands the samples to `percentiles`, prints `P50=… P95=… P99=…`. Surfaces scheduling drift without altering the drain loop.

## D — Dual-input

- **Status:** ⏳ not yet started.
- **There:** every CPAL driver opens the default mic into a single `Arc<Mutex<Vec<f32>>>`; the drain loop assumes one source per chunk.
- **Gap:** no abstraction over input stream; adding a second mic, OS loopback, or file source means duplicating the build-stream + thread dance per source.
- **Next:**
  - **Pure:** `pub trait AudioSource` in `src/audio/source.rs` where `start(...) -> SourceHandle` returns a handle that owns its own `mpsc::Sender<(ChunkOrigin, Vec<f32>)>` per source — each source owns its buffer, no shared `Arc<Mutex<Vec<f32>>>` coupling. Plus `pub enum ChunkOrigin { MicA, MicB, File, Loopback }` and `pub fn merge_sources(rx: Vec<mpsc::Receiver<(ChunkOrigin, Vec<f32>)>>, chunk_size: usize) -> impl Iterator<Item = (ChunkOrigin, Vec<f32>)>` that pulls in arrival order and yields one item per chunk. 4–6 tests covering single-source passthrough, interleaved arrivals, label preservation across merges, and the no-data timeout case.
  - **Manual CLI:** `cargo run -- dual-test` opens two sources concurrently (default mic + chosen device, or default mic + file reader) behind `AudioSource`, runs `merge_sources`, and prints `[mic-A] chunk #N samples=…` / `[mic-B] chunk #N samples=…` interleaved as they arrive.
  - **Refactor:** `audio::chunker::run_chunk_test` and `transcription::pipeline::run_mock_transcription_test` switch from opening the device themselves to consuming a merged-source iterator. As a side benefit, the build-stream + drain body that currently lives in three places (`chunker`, `pipeline`, and `recorder`) collapses into the `AudioSource` module — a real net-negative diff, not just a refactor. Pure helpers and their existing tests stay untouched.

## I — Interpretation

- **Status:** ⏳ not yet started.
- **There:** chunk-level transcription classifier exists (`mock_transcribe_chunk`) — no translation step.
- **Gap:** to deliver interpretation we need (a) source-language detection per chunk or per session, (b) target-language routing, (c) translation. None in tree yet.
- **Next:**
  - **Pure:** `pub trait Interpreter { fn interpret(&self, text: &str) -> Result<String> }`, `pub trait LanguageDetector { fn detect(&self, sample: &str) -> Option<Language> }`, `pub enum Language { Es, En, Pt, ... }` with `Default = En`, plus `pub fn pick_target(source: Language, requested: Option<Language>) -> Language` (defaults to `En` for non-`En` input). 6–8 unit tests covering detect/pick_target/identity passthroughs.
  - **Manual CLI:** `cargo run -- interpret-test` — 10 s capture, per chunk: `mock_transcribe_chunk` → `MockInterpreter` (returns `format!("[en→es] {text}")` for now) → print interpreted line per chunk. Reuses the existing chunking framework, only the per-chunk transform differs.

## S — System

- **Status:** ✅ CLI foundation shipped in v0.1.0; desktop UI deliberately deferred.
- **There:** 6 manual CLI modes (`devices`, `mic`, `mic-5s`, `record-5s`, `chunk-test`, `mock-transcribe`); `Cargo.toml` deps (`cpal = "0.18"`, `anyhow = "1.0.102"`, `hound = "3.5"`); `LICENSE` (MIT); `DEVELOPMENT_RULES.md` + `AGENTS.md` codify the testing policy; annotated `v0.1.0` tag pinned to the docs commit.
- **Gap:** desktop GUI (Tauri) and CI workflow are deferred until the audio + transcription + interpretation pipelines stabilise. CI is the next-tractable axis.
- **Next:**
  - `.github/workflows/ci.yml` running `cargo check` and `cargo test` on every push and PR — small, mechanical, lights the green badge.
  - Then: `cargo run` ergonomics — `--release`, `--quiet`, structured `--json` output. The pure helper for JSON emission is testable; the CLI flag plumbing is not.
  - Eventually: a Tauri desktop shell consuming the same `audio::chunker`, `transcription::{mock, real}`, and `interpretation::*` pipelines. Per `AGENTS.md`: no Tauri yet.

## How to pick up an axis

Per [`DEVELOPMENT_RULES.md`](./DEVELOPMENT_RULES.md), every new feature must:

1. Land pure logic as `#[cfg(test)] mod tests` next to the code that lives.
2. Skip CPAL / microphone / permission / physical-device tests entirely.
3. Ship a manual `cargo run -- <new-mode>` that proves the hardware layer end-to-end.

When picking up more than one axis, the order that composes best with the least churn is:

**T → D → I → S (CI) → S (Tauri) → R (reactive drain)**

T first because real transcription is the highest-signal unknown; D second because it composes with T; I third because it composes with both; S-spacing afterwards because each axis stabilises the previous. R's reactive-drain tightening is intentionally last — it's a perf refinement that only matters once the rest of the stack is real.
