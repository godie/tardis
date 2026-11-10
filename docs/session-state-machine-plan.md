# Plan: one state machine for the live session

Status: proposal, to be turned into GitHub issues.
Scope: `src/app/session_phase.rs`, `src-tauri/src/lib.rs`, `src/app/ui_events.rs`, `ui/`.

## Summary

Tardis can get stuck on "Listening" when the worker fails, because the session lifecycle lives in five copies that do not stay in sync. This plan makes the backend the single owner of the phase (`SessionPhase`) and reduces the UI to a render of that phase.

**The bug (read from the code, not reproduced).** `start_live_transcription` returns `Ok` as soon as the thread is spawned and the UI moves to `listening`. If the runner fails afterwards (microphone, Piper player, or `stages.run(...)?` in the loop), only an `error` event is emitted: never `stopped`. `SessionCleanupGuard` frees the backend, but `app.js` ignores `error` when deciding the phase. Start stays disabled, and Stop answers "No live transcription session is running" and sends the UI back to `listening`.

**The five copies of the lifecycle:** `SessionPhase` (Rust, unused), `sessionPhase` (JS), `is_running` + `stop_signal` in `LiveSessionState`, `MockAppState`, and `AppState.status`.

**What "don't break anything" means here:**

- The `status`, `transcript`, `output`, `speech` and `error` events keep their shape and labels; every new field is additive.
- The current specs in `ui/tests/session.spec.js` pass unedited.
- Each issue starts with the failing test and passes the validation matrix in `DEVELOPMENT_RULES.md` for the area it touches.
- Only the paths that get stuck today change behavior.

**Repo constraints the plan respects:** no async runtime, no new dependencies, static JS, mocks intact.

## Execution order

Issues 1 and 2 depend on nothing and can run in parallel; 1 stops the bug and 2 prepares the phase table. Issue 5 is the one that changes the UI and 9 closes with the docs.

| # | Issue | Depends on | Size | Area |
| --- | --- | --- | --- | --- |
| 1 | Emit a terminal event when the worker ends | — | S | `src-tauri` |
| 2 | Complete the `SessionPhase` transition table | — | S | `src/app` |
| 3 | `LiveSessionState` stores `SessionPhase` | 2 | M | `src-tauri` |
| 4 | `get_session_phase` command and additive `phase` field | 3 | S | `src-tauri`, `src/app` |
| 5 | UI renders `phase` and drops the JS mirror | 1, 4 | M | `ui` |
| 6 | Status pill driven by `data-phase` | 5 | S | `ui` |
| 7 | Single `data-state` in the secondary widgets | 5 | S | `ui` |
| 8 | Remove `MockAppState` and mock commands if unused | 5 | S | `src-tauri` |
| 9 | Update `AGENTS.md` and comments | 5 | S | docs |

## Issues

Each issue is written to be pasted into GitHub as is: title, labels, context, acceptance criteria as a checklist, and validation.

### 1. fix(tauri): emit a terminal event when the worker ends without `stopped`

**Labels:** `bug`, `tauri`, `priority:high` · **Depends on:** — · **Size:** S

**Context.** If `run_live_local_transcription_with_config_and_events` returns `Err` (for example from `MicInput::start_with_device`, `CpalPlayer::new` or `stages.run(...)?` in the loop), the thread emits only `AppEvent::Error`. The backend is freed by `SessionCleanupGuard`, but the UI never receives `stopped` and stays on `listening` with Start disabled.

**Change.** Guarantee one terminal event per session: after an `Err`, emit `Error` and then `StatusChanged(Stopped)`; the guard emits `Stopped` if the thread panics. On the happy path the runner already emits `Stopped`, so it must not be duplicated.

**Acceptance criteria**

- [ ] A failure before `Listening` (mic or player) ends with `error` and then `stopped`.
- [ ] A mid-session failure ends the same way.
- [ ] A worker panic emits `stopped` from the guard.
- [ ] The happy path emits exactly one `stopped`.
- [ ] Payloads and labels of existing events do not change.

**Validation.** `src-tauri/` row of the matrix: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cargo test --manifest-path src-tauri/Cargo.toml`. Unit test with a fake runner that returns `Err`. First step: reproduce the bug (for example, Start without microphone permission) and keep it as a test.

### 2. refactor(core): complete the `SessionPhase` transition table

**Labels:** `refactor`, `core`, `good first issue` · **Depends on:** — · **Size:** S

**Context.** `src/app/session_phase.rs` only reacts to `AppStatus`; `Starting` and `Stopping` exist as variants but nothing reaches them, and no code uses the module (only a comment in `app.js` says it "mirrors" it).

**Change.** Add an input type `SessionInput` (`StartAccepted`, `StartFailed`, `StopAccepted`, `Status(AppStatus)`, `WorkerExited`) and `apply(self, input) -> Result<SessionPhase, IllegalTransition>`. Not wired in yet; the module stays pure (no I/O, no Tauri, no CPAL).

**Acceptance criteria**

- [ ] Full table test: 4 phases × every input, illegal ones included.
- [ ] `WorkerExited` takes any phase to `Idle`.
- [ ] The module's current tests keep passing.
- [ ] Labels (`as_label`) unchanged.

**Validation.** `src/` row of the matrix.

### 3. refactor(tauri): `LiveSessionState` stores `SessionPhase` instead of `is_running`

**Labels:** `refactor`, `tauri` · **Depends on:** 2 · **Size:** M

**Context.** `is_running: Mutex<bool>` and `stop_signal: Mutex<Option<Arc<AtomicBool>>>` encode four states with two variables and a lock-order contract that has to be documented. `try_mark_running`, `take_stop_signal` and `reset_session` are transitions in disguise.

**Change.** Replace `is_running` with `Mutex<SessionPhase>`; `try_mark_running` becomes `apply(StartAccepted)` and the guard becomes `apply(WorkerExited)`. Keep `SESSION_ALREADY_RUNNING_ERR` and `NO_SESSION_RUNNING_ERR` verbatim. Review the lock-order contract, which should end up simpler, not more complex.

**Acceptance criteria**

- [ ] A double Start still returns the same error.
- [ ] A quick Start → Stop → Start does not let the new worker take the microphone before the old one finishes (behavior documented today).
- [ ] The existing `LiveSessionState` tests (the test block in `lib.rs`) pass without touching their assertions.
- [ ] A poisoned mutex recovers as it does today.

**Validation.** `src-tauri/` row of the matrix.

### 4. feat(tauri): `get_session_phase` command and additive `phase` field on status events

**Labels:** `enhancement`, `tauri`, `core` · **Depends on:** 3 · **Size:** S

**Context.** The UI currently guesses the phase by combining the `invoke` result with events, and can lose the race if `stopped` arrives before the `invoke` resolves.

**Change.** New `get_session_phase` command and a `phase: Option<String>` field on `UiAppEvent`, with `skip_serializing_if = "Option::is_none"`, set only on `status` events. Events that are not status events keep their shape.

**Acceptance criteria**

- [ ] The JSON of `transcript`, `output`, `speech` and `error` is identical to today's (the existing serialization test stays green).
- [ ] A `status` event includes `phase` as a lowercase label.
- [ ] `get_session_phase` matches the internal phase.

**Validation.** `src-tauri/` row of the matrix; tests in `ui_events.rs`.

### 5. refactor(ui): render `phase` and remove the mirrored table in `app.js`

**Labels:** `refactor`, `ui`, `bug`, `priority:high` · **Depends on:** 1, 4 · **Size:** M

**Context.** `app.js` reimplements the phase table by hand (`SESSION_PHASES`, `sessionPhase`, `isSessionLive` and four assignments). `case "error"` does not change the phase, and Stop's `catch` sends the UI back to `listening` even when the backend has no session.

**Change.** The UI stores the last `phase` it received and draws the controls from it. On startup, and after a Stop "No session" error, it calls `get_session_phase` to resync. The local state is deleted.

**Acceptance criteria**

- [ ] New spec: the worker emits `error` and `stopped` after Start → Start is enabled again and the panel unlocked.
- [ ] New spec: `stopped` arrives before Start's `invoke` resolves → the UI ends in `idle`.
- [ ] New spec: Stop answers "No live transcription session is running" → the UI resyncs to `idle`.
- [ ] The current specs pass unedited (including "a second click while starting does not spawn a second session").
- [ ] No change to the visible status texts.

**Validation.** `ui/` row of the matrix: `cargo fmt --check`, `clippy`, `cargo test`, `src-tauri` tests, `npm run test:e2e`.

### 6. refactor(ui): status pill driven by `data-phase` instead of text prefixes

**Labels:** `refactor`, `ui`, `a11y` · **Depends on:** 5 · **Size:** S

**Context.** The CSS uses `data-status^="Starting"` and `data-status="Listening"`, and `setStatus` capitalizes backend strings; "Starting…" with the ellipsis and "Starting" without it are two states for the same concept.

**Change.** Add `data-phase` to the pill and migrate the selectors in `styles.css`. The visible text and the `aria-label` do not change.

**Acceptance criteria**

- [ ] Before/after screenshots of the four phases show no visual difference.
- [ ] `#status-announcer` still announces `Status: <text>`.
- [ ] The current specs (`toHaveText("Stopping…")`, etc.) stay green.

**Validation.** `ui/` row of the matrix.

### 7. refactor(ui): single `data-state` in the secondary widgets

**Labels:** `refactor`, `ui` · **Depends on:** 5 · **Size:** S

**Context.** File transcription keeps `isTranscribing`, `dataset.state` and `disabled` as three mirrors of the same state; the spoken-audio line combines `hidden`, `data-state` and text. Neither is coordinated with the live session.

**Change.** One value per widget (`idle | busy | done | error`; `hidden | spoken`) from which the other attributes are derived.

**Open question:** whether the spoken-audio line should be cleared when `stopped` arrives (today it stays visible until the next Start). This issue does not change it; decide in a separate issue.

**Acceptance criteria**

- [ ] Double click and Enter in the path field still do not launch two transcriptions.
- [ ] No visible or text changes.
- [ ] Current specs green.

**Validation.** `ui/` row of the matrix.

### 8. chore(tauri): remove `MockAppState` and the mock commands if no longer used

**Labels:** `chore`, `tauri`, `tech-debt` · **Depends on:** 5 · **Size:** S

**Context.** `MockAppState` has its own three-variant `AppStatus` and feeds `get_app_status`, `start_mock_listening` and `stop_mock_listening`. It is another copy of the lifecycle. It was not checked whether `ui/` or the specs still call them.

**Change.** First `grep` in `ui/` and `ui/tests/`. If nothing uses them, delete them along with their tests and their registration in the `invoke_handler`; `syncInitialState` switches to `get_session_phase`. If something does use them, close the issue with a note on what blocks it.

**Acceptance criteria**

- [ ] Zero references to `MockAppState` or the three commands.
- [ ] The initial UI shows `Idle` as before.
- [ ] `get_mock_transcript` and `get_mock_translation` are untouched unless they depend on the deleted state.

**Validation.** `ui/` row of the matrix.

### 9. docs: update `AGENTS.md` and comments about the session phase

**Labels:** `documentation` · **Depends on:** 5 · **Size:** S

**Context.** `AGENTS.md` describes the UI event bridge but does not mention `SessionPhase`; the comment in `app.js` saying "mirrors session_phase.rs" becomes stale after issue 5.

**Change.** Add a short section to `AGENTS.md` with the phase table and the rule "the UI does not decide phases, it draws them"; delete stale comments; update the lock-order comment in `lib.rs`.

**Acceptance criteria**

- [ ] Cited links and command names exist.
- [ ] No test counts in docs (repo rule).

**Validation.** Docs: none; check that links resolve.


## Decisions (2026-11-10)

The execution order and scope above were adjusted by the owner:

- **#64 ships after #65** (worker-emit shape): the terminal `stopped` on failure is emitted by the worker closure right after the `Error` event; the runner's success-path emission stays and the CLI contract does not change. Immediate-restart acceptance is a Playwright spec, not a Rust one.
- **#67 keeps `phase` on status events only**; the existing serialization test grows the assertion, non-status kinds stay byte-identical, and the field is documented in `AGENTS.md` (issue #72's scope).
- **#71 was closed as "in-use"**: `get_app_status` feeds `syncInitialState()` and the saved device-id spec stubs it, so the deletion premise did not hold; cleanup becomes possible only after the #67/#68 migration.

## Out of scope and risks

**Out of scope:** adopting XState or another library (the repo requires static JS and no new dependencies without sign-off); touching `HalfDuplexGate`, `EndpointState` or `Decision`, which are already pure, explicit and tested; changing visible texts or styles.

**Risks:**

- The bug was confirmed by reading the code, not by running the app. The first step of issue 1 is to reproduce it and keep it as a test.
- Event ordering between the worker thread and Start's `invoke` is not guaranteed; issue 5 covers it with a spec, but the real race only shows up in the app.
- The Playwright suite uses a fake Tauri bridge: the Rust-side changes (issues 1, 3 and 4) are validated with Rust tests and a manual check in the real window.
- Issue 8 may turn out to be unnecessary if something still uses the mock commands.

**Sources:** repo analysis (`src/app/session_phase.rs`, `src-tauri/src/lib.rs`, `src/transcription/live_local.rs`, `ui/app.js`, `ui/tests/session.spec.js`, `AGENTS.md`, `DEVELOPMENT_RULES.md`).
