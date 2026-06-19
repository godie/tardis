# AGENTS.md

Operational handbook for AI agents working on this repo. Sibling of
`DEVELOPMENT_RULES.md` (which covers *what* to test) — this file covers
*how* to work here: build/test commands, layout, CPAL 0.18 gotchas,
commit style, working conventions, things NOT to do.

## Build + test

```sh
cargo check                                          # fast type-check only (no tests)
cargo test                                           # 33 unit tests
cargo run                                            # default = `devices`
cargo run -- devices                                 # list host + I/O devices, exit
cargo run -- mic                                     # capture until Ctrl+C
cargo run -- mic-5s                                  # capture 5 s, exit
cargo run -- record-5s                               # write 5 s WAV to output/mic_test.wav
cargo run -- chunk-test                              # 10 s of capture, 1 s chunks
cargo run -- mock-transcribe                         # 10 s, mock transcript per chunk
```

`cargo run -- <mode>` is the only way to exercise CPAL glue, mic
permission, and disk I/O. Unit tests never touch these. Always run
`cargo check && cargo test` before asking the user to commit.

## File layout

```
src/
  main.rs                  # CLI dispatcher (matches on args.nth(1))
  audio/
    mod.rs                 # re-exports: devices, mic, recorder, chunker, volume
    devices.rs             # print_device_info (pure print, no tests)
    mic.rs                 # CPAL driver (no tests)
    volume.rs              # 14 tests
    recorder.rs            # CPAL + hound driver (no tests)
    chunker.rs             # 11 tests + CPAL capture loop (no tests)
  transcription/
    mod.rs                 # re-exports: mock, pipeline
    mock.rs                # 8 tests
    pipeline.rs            # CPAL driver (no tests)
```

When you add a module, mirror an existing one above:
- **Pure helpers** live in their own file with `#[cfg(test)] mod tests`
  so boundary behaviour is unit-testable.
- **CPAL/file I/O glue** that calls them sits next to them with *no*
  tests, exercised only by `cargo run -- <mode>`.

When you change file structure, update `DEVELOPMENT_RULES.md`'s
existing-coverage table in the same commit.

## CPAL 0.18 gotchas (verified during prior turns)

- `cpal::StreamConfig::sample_rate` is `u32` **directly**, not a tuple
  struct. Do not write `config.sample_rate.0` — fails with
  *"u32 is a primitive type and therefore doesn't have fields"*.
- `cpal::StreamConfig` is `Copy`. Pass it via `*config` to
  `device.build_input_stream(...)`.
- `device.name()` is gone in CPAL 0.18. Use `{}` / `to_string()`
  because `Device` implements `Display`.
- `host.input_devices()` and `host.output_devices()` return
  `Result<Devices, _>` — propagate with `?`, do not `.unwrap()`.
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
  the assertion to make it pass — confirm the behaviour is correct
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

## Things NOT to do

- ❌ `device.name()` — gone in CPAL 0.18. Use `{}` / `to_string()`.
- ❌ `config.sample_rate.0` — it's a plain `u32` in 0.18.
- ❌ Allocate, print, or block long in the audio callback.
- ❌ `.unwrap()` `host.input_devices()` / `output_devices()` /
  `device.default_input_config()`. `?` with a clear `anyhow!` /
  `Context` chain.
- ❌ Unit test CPAL streams, mic hardware, or permissions. Split
  logic into pure helpers, test those, then verify the rest manually
  via `cargo run -- <mode>`.
- ❌ Commit `output/mic_test.wav`. `.gitignore` excludes `/output`.
- ❌ Add `Todo: …` code paths, sneak in `unwrap()`-filled stubs, or
  leave dead `#[allow(dead_code)]` on functions that should be
  removed rather than silenced.
- ❌ Add Tauri / desktop UI, async runtime, or a real transcription
  API unless the user explicitly asks.
- ❌ Order `mod audio;` twice in `main.rs` — once led to
  `audio is defined multiple times` already.
