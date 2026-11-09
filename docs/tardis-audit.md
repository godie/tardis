What this repo does: TARDIS captures speech, transcribes it, translates it through mock or LLM providers, and can speak the result. It ships a Rust CLI, a Tauri desktop shell, local model containers, and a scheduled GitHub-issue pickup script that can run an autonomous coding agent.

Load assumed: one local user and one live session at a time, with utterances up to 60 seconds; Whisper and Piper may run on CPU. The scheduled issue trigger may also be started more than once, including after a stale lock.

Scope: audited `tardis/`, including the four currently untracked agent-trigger files. The sibling `tardisv1-ui/` is outside this audit. No code was changed.

## Must fix

1. **Give release write access only to the publisher** (`.github/workflows/release.yml:L27-28, L39-45, L118-124`)
   - **What this is:** The workflow grants `contents: write` at the top level, so every platform build job inherits it.
   - **Problem:** Build jobs run third-party actions and compile dependencies, but only the final release job needs to upload assets. Compromised build code could use the checkout token to change repository contents or releases.
   - **Fix:** Set workflow permissions to `contents: read`; grant `contents: write` only to the `release` job.
   - **If we skip it:** A compromised build step gets more repository access than it needs.

2. **Stop the installer from launching the backlog agent** (`scripts/install-agent-trigger.sh:L28-33`, `scripts/agent-backlog-pickup.sh:L76, L93-120`)
   - **What this is:** The installer runs the pickup script with `--help` as a check.
   - **Problem:** The pickup script does not handle `--help`; it syncs labels, claims a ready issue, and can run `opencode run --auto`. Installing the trigger can therefore start work and open a PR, despite `RunAtLoad` being disabled.
   - **Fix:** Remove the probe or add a help path that exits before any network, label, lock, or agent action. Add a test that proves the installer makes no pickup calls.
   - **If we skip it:** Installing launchd can unexpectedly claim a ticket and modify the repository.

3. **Fail closed before speaking respond-mode answers** (`src/processor/decision.rs:L149-168`, `src/processor/guard.rs:L41-84`, `src/transcription/live_local.rs:L623-664`)
   - **What this is:** Respond mode answers named questions and speaks replies that pass a short list of word checks.
   - **Problem:** `Ana, can we work together?` is classified as an answer; a reply such as `Sí, hagámoslo.` passes the reply guard and reaches synthesis. That can speak an agreement even though the mode promises not to make commitments.
   - **Fix:** Keep respond-mode text on screen and require explicit approval before speaking it, until the guard can safely reject commitment language.
   - **If we skip it:** A model can speak an unapproved agreement in the user's voice.

4. **Keep saved transcripts private on disk** (`src-tauri/src/lib.rs:L625-635, L687-719`, `src/config.rs:L63-67`)
   - **What this is:** The history file stores transcripts and rendered replies in plain text.
   - **Problem:** The file and `output` directory use default Unix permissions. With a common `022` umask in a directory other users can traverse, they are typically readable by every local account.
   - **Fix:** Store history in a private per-user app-data directory and restrict the directory/file to the owner (for example, `0700`/`0600` on Unix).
   - **If we skip it:** Other local users may read private meeting speech and translations.

5. **Resolve desktop audio and history paths outside the working directory** (`src/config.rs:L58-67`, `src/transcription/live_local.rs:L198-201`, `src-tauri/src/lib.rs:L286-289`)
   - **What this is:** The live WAV directory and history file use relative `output/...` paths.
   - **Problem:** A packaged app launched from a read-only or unrelated working directory can fail while creating `output/live_chunks`, before listening starts. History writes can also fail silently from the user's view because the error goes only to stderr.
   - **Fix:** Resolve a writable per-user app-data path in the Tauri shell and pass it to the live runner and history writer; keep the CLI's current relative paths.
   - **If we skip it:** The desktop app may fail to start a session or lose history outside a developer checkout.

6. **Emit `stopped` after worker cleanup on every exit** (`src-tauri/src/lib.rs:L272-321`, `src/transcription/live_local.rs:L451-460`, `ui/app.js:L442-489`)
   - **What this is:** The frontend unlocks only on `stopped`; the worker's cleanup guard resets backend state.
   - **Problem:** On error the worker emits only `error`, leaving the UI locked after its stop signal is cleared. On normal exit the runner emits `stopped` before the cleanup guard resets state, so an immediate Start can race the old worker.
   - **Fix:** Have the worker reset state, then emit exactly one terminal `stopped` event on every exit; add tests for failure and immediate restart.
   - **If we skip it:** One startup or runtime error can leave the shell unusable until it is restarted.

## Should fix

7. **Set a Whisper timeout that supports the configured utterance size** (`src/transcription/local_whisper.rs:L144-172`, `src/app/config.rs:L203-205`, `src/transcription/live_local.rs:L355-374`, `src-tauri/src/lib.rs:L330-355`, `docker/faster-whisper/README.md:L118-124`)
   - **What this is:** Local Whisper uses a blocking HTTP request without an explicit timeout; reqwest 0.13.4 defaults to 30 seconds.
   - **Problem:** The app accepts 60-second utterances, while the documented CPU model can take 2–5 times the audio length. A valid long utterance can time out before transcription finishes, and Stop waits for the in-flight request.
   - **Fix:** Set a timeout based on the supported model and utterance bound, and make Stop cancellation behavior explicit; test with a delayed local server.
   - **If we skip it:** Long CPU transcriptions become errors, and Stop may appear stuck for up to the request limit.

8. **Cap response bytes from local model servers** (`src/transcription/local_whisper.rs:L157-172`, `src/synthesis/local_http.rs:L54-74`)
   - **What this is:** Whisper reads full JSON/error bodies and Piper buffers the full WAV body before parsing.
   - **Problem:** A broken local service that returns a very large body can make the app allocate that entire response in memory; the LLM path already has a 64 KB cap, but these paths do not.
   - **Fix:** Read only up to a documented response limit and return a clear over-limit error.
   - **If we skip it:** A faulty or compromised local endpoint can exhaust app memory.

9. **Run Tauri unit tests in CI** (`.github/workflows/ci.yml:L41-50`)
   - **What this is:** CI lints and checks the Tauri crate but runs only the root crate's `cargo test`.
   - **Problem:** Assertions for Tauri session cleanup and history writes never run in CI, so regressions can pass the pull-request gate.
   - **Fix:** Add `cargo test --manifest-path src-tauri/Cargo.toml` to the check job.
   - **If we skip it:** The separate Tauri test suite can fail after merge without blocking a pull request.

10. **Make stale-lock recovery atomic** (`scripts/agent-backlog-pickup.sh:L78-91`)
    - **What this is:** The pickup script removes a lock after reading a dead PID, then creates a new lock.
    - **Problem:** Two starts can both read the same stale PID; one can create a fresh lock and the other can then delete it using its stale read. Both runs may claim and process the same issue.
    - **Fix:** Serialize stale-lock recovery with an atomic reaper lock or replace the PID-directory scheme with a reliable OS lock.
    - **If we skip it:** Concurrent autonomous sessions can race on the same branch or pull request.

11. **Scope and paginate backlog discovery** (`scripts/agent-backlog-pickup.sh:L50-71, L93-104`)
    - **What this is:** Board sync reads only 50 project items, drops each item's repository name, and the issue query considers only 10 results before sorting.
    - **Problem:** A shared project can cause issue `other-repo#5` to be checked or labeled as `tardis#5`; a ready issue beyond either limit can be missed, despite the script promising the lowest-numbered ready ticket.
    - **Fix:** Filter or pass the repository for every board item, paginate both sources, then select the lowest number from the complete TARDIS queue.
    - **If we skip it:** Tickets can be mislabeled, skipped, or picked in an order the docs do not promise.

12. **Quote paths in the launchd job** (`scripts/install-agent-trigger.sh:L39-62`)
    - **What this is:** The installer places raw repository paths inside a shell command string and XML elements.
    - **Problem:** A checkout path with spaces is split by `zsh`; characters such as `&` can also make the generated plist invalid.
    - **Fix:** Pass the runner as a direct `ProgramArguments` entry and use launchd log-path keys, with XML-escaped path values.
    - **If we skip it:** Installations from common paths such as `~/Work Projects/tardis` can fail to run.

13. **Remove transcript payloads from debug logging** (`ui/app.js:L49, L440-445`)
    - **What this is:** `DEV` is hardcoded to `true`, and the event handler logs every full `app-event` payload.
    - **Problem:** Transcript, output, and speech text are printed to the webview console, contrary to `AGENTS.md`'s rule not to log them beyond the history file.
    - **Fix:** Remove the payload log or gate it behind a development-only build flag that is off in release bundles.
    - **If we skip it:** Anyone inspecting or collecting the webview console can see live conversation text.

14. **Correct the translation status in the README** (`README.md:L279-285, L348`, `src/processor/llm.rs:L281-313`)
    - **What this is:** The README says TARDIS cannot run a real translation backend and lists it as a future release requirement.
    - **Problem:** The `llm` processor already sends translation requests to an OpenAI-compatible endpoint, and the README documents that provider elsewhere.
    - **Fix:** Remove or narrow the stale bullet and update the release checklist to describe what is still missing.
    - **If we skip it:** Users and release planning may treat an existing feature as unfinished.

## Nice to have

15. **Reuse the Whisper HTTP client across utterances** (`src/transcription/local_whisper.rs:L82-95, L124-146`)
    - **What this is:** The provider builds a new reqwest client for each utterance.
    - **Problem:** A long session repeatedly loses connection-pool reuse and creates avoidable setup work.
    - **Fix:** Construct the client once in `LocalWhisperClient` and reuse it.
    - **If we skip it:** Live Whisper works, but repeated utterances pay needless connection setup cost.

16. **Pin Docker model-server dependencies** (`docker/faster-whisper/docker-compose.yml:L23-24`, `docker/piper/Dockerfile:L7-11`)
    - **What this is:** The Whisper image uses a `latest-cpu` tag, and Piper installs an unpinned Python package on a mutable base image.
    - **Problem:** Rebuilding the same checkout later can fetch different server software and fail or change behavior.
    - **Fix:** Pin image digests and the Piper package version for reproducible local runs.
    - **If we skip it:** A previously working setup can change without a repository diff.

17. **Align the documented review-round cap** (`docs/agent-backlog-trigger.md:L13-15`, `scripts/agent-backlog-pickup.sh:L117-118`)
    - **What this is:** The docs say Qodo review stops after two fix rounds, while the agent prompt allows three rounds.
    - **Problem:** Operators cannot tell which cap the automation follows.
    - **Fix:** Use one number in both places.
    - **If we skip it:** The run may continue one round beyond the documented limit.

## Checks

Passed locally: `cargo fmt --check`, `cargo check`, both crates' Clippy and test commands, `npm run test:e2e`, syntax checks for the three agent-trigger scripts, and `scripts/install-skills.test.sh`.

Verdict: Fix findings 1–6 before external release or enabling the scheduled trigger; then address 7–13 before relying on live or unattended operation.

Not checked: packaged Tauri launch, real microphone permissions, Docker/model-server behavior, signed cross-platform bundles, or a full visual/accessibility review of the UI. CI job behavior was reviewed from configuration, not executed on GitHub.
