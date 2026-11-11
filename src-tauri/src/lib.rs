use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{Emitter, Manager};
use tardis::app::session_phase::{IllegalTransition, SessionInput, SessionPhase};

const SESSION_ALREADY_RUNNING_ERR: &str =
    "A live transcription session is already running. Stop it first.";
const NO_SESSION_RUNNING_ERR: &str = "No live transcription session is running.";

const MOCK_TRANSCRIPT: &str = "mock transcript: speech detected";
const MOCK_TRANSLATION: &str = "[mock es] mock transcript: speech detected";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum AppStatus {
    #[default]
    Idle,
    Listening,
    Stopped,
}

impl AppStatus {
    fn as_label(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Listening => "Listening",
            Self::Stopped => "Stopped",
        }
    }
}

#[derive(Debug, Default)]
struct MockAppState {
    status: AppStatus,
}

impl MockAppState {
    fn start(&mut self) -> &'static str {
        self.status = AppStatus::Listening;
        self.status.as_label()
    }

    fn stop(&mut self) -> &'static str {
        self.status = AppStatus::Stopped;
        self.status.as_label()
    }

    fn status(&self) -> &'static str {
        self.status.as_label()
    }
}

/// Shared state for the active live transcription session.
///
/// Managed by Tauri as an [`Arc`] so the background worker thread
/// can hold its own clone and apply [`SessionInput::WorkerExited`] on
/// exit (success, error, or panic) without needing to round-trip
/// through the command handler.
///
/// **Lock order:** the two fields are never held at the same time.
/// A transition holds `phase` only for the duration of
/// [`SessionPhase::apply`], the stop-signal helpers hold
/// `stop_signal` only to store, take or clear it, and
/// [`LiveSessionState::reset_session`] releases `phase` before it
/// locks `stop_signal`. A concurrent `stop_live_transcription` call
/// therefore cannot deadlock with the worker-thread cleanup.
///
/// Field-level rationale:
/// - `stop_signal: Mutex<Option<Arc<AtomicBool>>>` — the worker
///   polls this `AtomicBool` each tick. Holding the `Arc` (vs a
///   raw `bool`) lets the UI thread reach the same atomic from
///   `stop_live_transcription` while the worker reads it.
///   `Option` + clearing it on cleanup avoids stale-flag
///   confusion between sequential sessions.
/// - `phase: Mutex<SessionPhase>` — the session lifecycle that a
///   separate `is_running` bool used to encode.
///   [`SessionInput::StartAccepted`] takes the session slot and
///   [`SessionInput::WorkerExited`] releases it; every other pair is
///   an illegal transition. One named value keeps the re-entry guard
///   and the phase read by the UI on the same source.
#[derive(Debug)]
pub struct LiveSessionState {
    stop_signal: Mutex<Option<Arc<AtomicBool>>>,
    phase: Mutex<SessionPhase>,
}

impl Default for LiveSessionState {
    fn default() -> Self {
        Self {
            stop_signal: Mutex::new(None),
            phase: Mutex::new(SessionPhase::starting_phase()),
        }
    }
}

impl LiveSessionState {
    /// Lock the phase field, recovering a poisoned mutex.
    ///
    /// Recovery keeps the behaviour the bool had: the field only ever
    /// holds a [`SessionPhase`], so a panic mid-transition cannot leave
    /// a half-updated value behind.
    fn lock_phase(&self) -> MutexGuard<'_, SessionPhase> {
        self.phase
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The phase the session is currently in.
    pub fn phase(&self) -> SessionPhase {
        *self.lock_phase()
    }

    /// Returns `true` while a session owns the microphone slot.
    ///
    /// [`SessionPhase::Starting`] counts: the worker exists from the
    /// accepted start, before the runner reports listening.
    pub fn is_session_running(&self) -> bool {
        self.phase() != SessionPhase::Idle
    }

    /// Apply one lifecycle input to the stored phase.
    ///
    /// An illegal transition leaves the phase untouched and reports it,
    /// so a second Start while one owns the slot is a no-op plus
    /// [`SESSION_ALREADY_RUNNING_ERR`] at the call site. Tolerates a
    /// poisoned mutex by recovering the inner value.
    pub fn apply(&self, input: SessionInput) -> Result<(), IllegalTransition> {
        let mut phase = self.lock_phase();
        *phase = phase.apply(input)?;
        Ok(())
    }

    /// Store the worker's `Arc<AtomicBool>` so the
    /// `stop_live_transcription` command can reach it.
    pub fn store_stop_signal(&self, signal: Arc<AtomicBool>) {
        *self
            .stop_signal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(signal);
    }

    /// Take the `Arc<AtomicBool>` (if any), leaving `None` in its
    /// place. Returned so the caller can flip it.
    pub fn take_stop_signal(&self) -> Option<Arc<AtomicBool>> {
        self.stop_signal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    /// Return the session to [`SessionPhase::Idle`] and drop the stop
    /// signal. Invoked by [`SessionCleanupGuard`] when the worker thread
    /// exits for any reason (return, error, or panic).
    ///
    /// `WorkerExited` is legal from every phase, so the transition cannot
    /// fail; it happens before the signal is cleared, and the phase lock is
    /// already released by then (see the lock order above).
    pub fn reset_session(&self) {
        let _ = self.apply(SessionInput::WorkerExited);
        *self
            .stop_signal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

fn lock_state<'a>(state: &'a tauri::State<'_, Mutex<MockAppState>>) -> MutexGuard<'a, MockAppState> {
    match state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// ===== New live transcription commands ====================================

/// RAII guard that returns a [`LiveSessionState`] to “idle” when
/// the worker thread exits, **including via panic**.
///
/// The guard holds an [`Arc<LiveSessionState>`] (cloned from the
/// Tauri managed state) and runs [`LiveSessionState::reset_session`]
/// in its [`Drop`] impl: it applies [`SessionInput::WorkerExited`]
/// and clears the stop signal. Because [`Drop`] always runs on scope
/// exit (normal return, `Err`, or unwind), the session state cannot be
/// left in a stuck “running” state even if the worker panics inside
/// the capture loop or the provider closure.
///
/// `reset_session` never holds the phase and the stop signal at the
/// same time, so this path cannot deadlock with a concurrent
/// `stop_live_transcription` call.
struct SessionCleanupGuard(Arc<LiveSessionState>);

impl Drop for SessionCleanupGuard {
    fn drop(&mut self) {
        self.0.reset_session();
    }
}

/// Persist one backend event, then emit it to the frontend.
///
/// Order matters: anything the user can already see is already on disk.
/// Infallible on purpose: [`append_history`] reports its own failures and
/// must not swallow the event, and an emit failure on the worker thread has
/// no recovery path.
fn emit_app_event(app: &tauri::AppHandle, event: &tardis::app::events::AppEvent) {
    append_history(event, Path::new(tardis::config::HISTORY_PATH));
    let _ = app.emit(
        "app-event",
        tardis::app::ui_events::app_event_to_ui_event(event),
    );
}

/// The extra events the worker emits once the runner call has resolved.
///
/// `None` is the runner's success path, which emits
/// `StatusChanged(Stopped)` itself: adding nothing there is what keeps a
/// session at exactly one terminal event. `Some(message)` is a failure,
/// either before capture started (microphone or player set-up) or inside
/// the loop; that path emitted at most a per-chunk `error` and no terminal
/// status, so the worker owes the user the top-level error and then the
/// `stopped` that re-enables Start.
fn worker_exit_events(error: Option<String>) -> Vec<tardis::app::events::AppEvent> {
    use tardis::app::events::{AppErrorEvent, AppEvent, AppStatus};

    error
        .map(|message| {
            vec![
                AppEvent::Error(AppErrorEvent { message }),
                AppEvent::StatusChanged(AppStatus::Stopped),
            ]
        })
        .unwrap_or_default()
}

/// Start a live transcription session in the background.
///
/// Spawns a thread that captures audio from the default microphone,
/// routes each speech-like chunk through the selected provider, and
/// emits `app-event` Tauri events to the frontend for every
/// [`tardis::app::events::AppEvent`] produced.
///
/// **Default provider is `"mock-local"` (no Docker required).** Pass
/// `"local-whisper"` to use the self-hosted faster-whisper server.
///
/// **Re-entry safety:** if a session is already running this command
/// returns [`SESSION_ALREADY_RUNNING_ERR`]. The state is registered
/// with Tauri as `Arc<LiveSessionState>` so the worker thread holds
/// its own clone and applies [`SessionInput::WorkerExited`] on exit
/// (success, error, or panic) via [`SessionCleanupGuard`]. Once the
/// guard fires, the user can call `start_live_transcription` again
/// without first calling `stop_live_transcription`.
///
/// **Validation order:** the provider name is validated **before**
/// any state mutation, so an unknown provider does **not** take the
/// session slot and strand the app in an unrecoverable state.
#[tauri::command]
fn start_live_transcription(
    app: tauri::AppHandle,
    session: tauri::State<'_, Arc<LiveSessionState>>,
    config: Option<tardis::app::config::AppRuntimeConfig>,
) -> Result<String, String> {
    let session: Arc<LiveSessionState> = session.inner().clone();

    // 1. Build + normalize + validate the config BEFORE touching
    //    any state. If validation fails, return early and leave
    //    the phase untouched. The UI sends a config it built
    //    from the settings panel; on `None` we fall back to
    //    `AppRuntimeConfig::default()` so old frontends (or CLI
    //    smoke tests calling the same command) keep working.
    let config =
        tardis::app::config::normalize_runtime_config(config.unwrap_or_default());
    tardis::app::config::validate_runtime_config(&config).map_err(|e| e.to_string())?;

    // 2. Take the session slot. The transition is legal only from
    //    `Idle`, which is what makes a double Start return the
    //    documented error without a second boolean to keep in sync.
    if session.apply(SessionInput::StartAccepted).is_err() {
        return Err(SESSION_ALREADY_RUNNING_ERR.to_string());
    }

    // 3. Allocate a fresh stop signal and store it.
    let stop_signal = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop_signal);
    session.store_stop_signal(Arc::clone(&stop_signal));

    // 4. Spawn the worker. The cleanup guard resets state when
    //    the closure (or anything it awaits) returns OR panics.
    let app_clone = app.clone();
    let provider_label = config.transcription_provider.clone();
    let config_clone = config.clone();
    let session_for_cleanup = Arc::clone(&session);

    // We use `Builder::spawn` (rather than `std::thread::spawn`)
    // because `Builder::spawn` returns `io::Result<JoinHandle<T>>`,
    // which lets us free the session slot and `stop_signal` if the
    // OS refuses to create the thread (resource limits, etc.). Plain
    // `std::thread::spawn` would instead panic in the parent thread
    // on the same condition, leaving the phase stuck in `Starting`
    // forever and stranding every subsequent Start behind a guard
    // that never runs.
    //
    // `.name(...)` lets crash dumps and `ps -L` listings show a
    // recognisable thread identity; cheap insurance.
    std::thread::Builder::new()
        .name("tardis-live-transcription-worker".to_string())
        .spawn(move || {
            // Held for the entire worker lifetime. When this scope
            // exits (normally or via panic), `Drop` resets the shared
            // session state so the next `start_live_transcription`
            // call is not blocked.
            let _cleanup = SessionCleanupGuard(session_for_cleanup);

            let result = tardis::transcription::live_local::run_live_local_transcription_with_config_and_events(
                config_clone,
                None, // None => run until stop signal is set
                Some(stop_clone),
                move |event| emit_app_event(&app_clone, &event),
            );

            // The runner emits `stopped` itself on its success path, so a
            // clean exit pins nothing here and a session sees exactly one
            // terminal event. A failed one has emitted no terminal status at
            // all, and the UI unlocks Start only on `stopped`.
            let error = result
                .err()
                .map(|e| format!("Live transcription failed: {e}"));
            for event in worker_exit_events(error) {
                emit_app_event(&app, &event);
            }
        })
        // Spawn-failure rollback: if thread creation fails the
        // cleanup-guard never gets to fire (the closure never
        // started), so we must free the session slot and clear
        // `stop_signal` ourselves before returning the error to the
        // frontend.
        .map_err(|e| {
            session.reset_session();
            format!("Failed to spawn live transcription worker thread: {e}")
        })?;

    // Slightly more informative than the spec's literal "started"
    // so the UI / CLI logs can tell at a glance which provider was
    // loaded. The frontend only logs this string; no contract
    // depends on the exact wording.
    Ok(format!("started with provider {provider_label}"))
}

/// Signal the active live transcription session to stop.
///
/// Takes the `Arc<AtomicBool>` from [`LiveSessionState`], flips it
/// to `true`, and returns `"stopping"`. The worker polls the flag
/// each tick; on noticing it, the runner returns and the
/// [`SessionCleanupGuard`] then applies [`SessionInput::WorkerExited`]
/// (phase back to `Idle`) and clears `stop_signal` so a subsequent
/// `start_live_transcription` can succeed.
///
/// **Intentionally does not touch the phase.** The worker is still
/// tearing down CPAL resources when `stop` returns. Releasing the
/// session slot here would race a fast Start→Stop→Start cycle, where
/// the new worker could grab the microphone while the old one is
/// still releasing it.
#[tauri::command]
fn stop_live_transcription(
    session: tauri::State<'_, Arc<LiveSessionState>>,
) -> Result<String, String> {
    let session: Arc<LiveSessionState> = session.inner().clone();
    match session.take_stop_signal() {
        Some(signal) => {
            signal.store(true, Ordering::Relaxed);
            Ok("stopping".to_string())
        }
        None => Err(NO_SESSION_RUNNING_ERR.to_string()),
    }
}

/// List the input devices the host reports, for the settings selector.
///
/// Each entry carries a stable `id` (what the frontend persists in
/// `runtime.json`, resolved back through `cpal`'s `Host::device_by_id`) and a
/// human `name` (what the dropdown shows). Touches CPAL, so it is not
/// unit-tested (DEVELOPMENT_RULES rule 2); the parsing and fallback rules it
/// stands on live in [`tardis::audio::capture`] and are covered there.
#[tauri::command]
fn list_input_devices() -> Result<Vec<tardis::audio::capture::InputDeviceInfo>, String> {
    tardis::audio::capture::list_input_devices().map_err(|e| e.to_string())
}

/// Return the list of supported transcription provider names.
///
/// Pure: no I/O, no state access. Used by the frontend to
/// populate the provider selector. Strings are sourced from
/// [`tardis::app::config::supported_transcription_providers`]
/// (kept in lockstep with `start_live_transcription`'s
/// validator) so adding a new provider requires editing exactly
/// one place.
#[tauri::command]
fn get_supported_transcription_providers() -> Vec<String> {
    tardis::app::config::supported_transcription_providers()
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Return the backend's list of supported process-stage names (`mock`,
/// `llm`). Mirrors [`get_supported_transcription_providers`] so the
/// settings panel rebuilds both dropdowns from one source of truth.
#[tauri::command]
fn get_supported_processor_providers() -> Vec<String> {
    tardis::app::config::supported_processor_providers()
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Return the default [`tardis::app::config::AppRuntimeConfig`]
/// the UI should pre-populate the settings panel with on first
/// load.
///
/// Pure: no I/O, no state access. The defaults are sourced from
/// centralised [`tardis::config`] constants so the frontend and
/// the Tauri command agree on the same starting point without
/// the UI duplicating constant values. The frontend is expected
/// to clone + mutate this for the session; the backend
/// re-validates any field the user changes before starting a
/// session.
#[tauri::command]
fn get_default_runtime_config() -> tardis::app::config::AppRuntimeConfig {
    tardis::app::config::AppRuntimeConfig::default()
}

/// Load the persisted [`tardis::app::config::AppRuntimeConfig`]
/// from `<OS config dir>/<tardis::runtime.json>`.
///
/// Resolution paths per platform (handled by Tauri's
/// [`tauri::Manager::path`]):
/// - **Linux**: `$XDG_CONFIG_HOME/tardis/runtime.json` (or
///   `~/.config/tardis/runtime.json`).
/// - **macOS**: `~/Library/Application Support/tardis/runtime.json`.
/// - **Windows**: `%APPDATA%\tardis\runtime.json`.
///
/// **First-run behavior:** if the file does not exist, returns
/// [`AppRuntimeConfig::default`] so the UI falls back to the
/// same defaults [`get_default_runtime_config`] returns. A
/// corrupt or out-of-bounds file surfaces the underlying
/// [`tardis::app::settings_store::SettingsStoreError`] as a
/// user-facing string. The command does **not** delete a corrupt
/// file; the next `save_runtime_settings` call overwrites it.
#[tauri::command]
fn load_runtime_settings(
    app: tauri::AppHandle,
) -> Result<tardis::app::config::AppRuntimeConfig, String> {
    let base_dir = app
        .path()
        .config_dir()
        .map_err(|e| format!("Could not resolve OS config dir: {e}"))?;
    let path = tardis::app::settings_store::settings_file_path(&base_dir);
    tardis::app::settings_store::load_from_path_or_default(&path)
        .map_err(|e| e.to_string())
}

/// Persist the [`tardis::app::config::AppRuntimeConfig`] to
/// `<OS config dir>/<tardis>/runtime.json`. Normalizes and
/// validates the input before writing; an invalid config is
/// returned as `Err` **without** touching the disk.
///
/// The write is **atomic** (write-then-rename via
/// [`tardis::app::settings_store::save_to_path`]) so a crash
/// mid-write either leaves the previous file untouched or
/// replaces it cleanly. The parent `<OS config dir>/<tardis>/`
/// directory is auto-created on first save.
///
/// Called by the frontend on every settings-input change so the
/// user's selections survive Tauri shell restarts.
#[tauri::command]
fn save_runtime_settings(
    app: tauri::AppHandle,
    config: tardis::app::config::AppRuntimeConfig,
) -> Result<(), String> {
    let config = tardis::app::config::normalize_runtime_config(config);
    tardis::app::config::validate_runtime_config(&config).map_err(|e| e.to_string())?;
    let base_dir = app
        .path()
        .config_dir()
        .map_err(|e| format!("Could not resolve OS config dir: {e}"))?;
    let path = tardis::app::settings_store::settings_file_path(&base_dir);
    tardis::app::settings_store::save_to_path(&path, &config)
        .map_err(|e| e.to_string())
}

// ===== Existing mock / file-transcribe commands ===========================

#[tauri::command]
fn get_app_status(state: tauri::State<'_, Mutex<MockAppState>>) -> String {
    lock_state(&state).status().to_string()
}

#[tauri::command]
fn start_mock_listening(state: tauri::State<'_, Mutex<MockAppState>>) -> String {
    lock_state(&state).start().to_string()
}

#[tauri::command]
fn stop_mock_listening(state: tauri::State<'_, Mutex<MockAppState>>) -> String {
    lock_state(&state).stop().to_string()
}

#[tauri::command]
fn get_mock_transcript() -> String {
    MOCK_TRANSCRIPT.to_string()
}

#[tauri::command]
fn get_mock_translation() -> String {
    MOCK_TRANSLATION.to_string()
}

// ===== Pure helpers =======================================================
//
// Both helpers are pure (no FS, no HTTP, no Tauri state) so they can
// be unit-tested in isolation. They sit next to the Tauri command
// they back because at this stage the file is still small and
// avoiding premature modularization keeps the integration
// discoverable. If a second Tauri file-writing or HTTP-backing
// command lands, lift these into a dedicated module.

/// Validate a user-supplied WAV file path before it reaches the
/// provider.
///
/// Returns the trimmed input on success. Rejects empty or
/// whitespace-only inputs with a user-facing error string. Does NOT
/// check whether the file actually exists on disk; the provider's
/// own `path.is_file()` check surfaces the file-not-found case with
/// a more specific message ("WAV file does not exist: <path>").
///
/// Pure: no FS or HTTP access.
pub fn validate_wav_path_input(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("Please enter a WAV file path.".to_string());
    }
    Ok(trimmed.to_string())
}

/// Map a `LocalWhisperClient` anyhow error string into a more
/// user-facing message while preserving the underlying detail
/// (provider URL, HTTP status, response body).
///
/// The provider's error strings
/// ([`tardis::transcription::local_whisper::LocalWhisperClient::transcribe_wav_file`])
/// are stable and matched by keyword:
///
/// | Keyword(s)                          | Branded message prefix                     |
/// |-------------------------------------|--------------------------------------------|
/// | `"does not exist"`                  | `"WAV file not found."`                    |
/// | `"sending POST"` + `"docker"`       | `"Could not reach the local faster-whisper server."` |
/// | `"returned status"`                 | `"Server rejected the WAV."`               |
/// | `"missing"` + `"text"`              | `"Server response had no transcript text."` |
/// | `"not a string"`                    | `"Server response format was unexpected."` |
/// | anything else                       | `"Transcription failed."`                  |
///
/// Pure: operates on a string. Matching is case-insensitive.
pub fn normalize_local_transcription_error(error: &str) -> String {
    let lower = error.to_lowercase();

    if lower.contains("does not exist") {
        return format!("WAV file not found. ({})", error);
    }
    if lower.contains("sending post") && lower.contains("docker") {
        return format!(
            "Could not reach the local faster-whisper server. ({})",
            error
        );
    }
    if lower.contains("returned status") {
        return format!("Server rejected the WAV. ({})", error);
    }
    if lower.contains("missing") && lower.contains("text") {
        return format!("Server response had no transcript text. ({})", error);
    }
    if lower.contains("not a string") {
        return format!("Server response format was unexpected. ({})", error);
    }

    format!("Transcription failed. ({})", error)
}

// ===== Tauri command ======================================================

/// Transcribe a WAV file on disk through the existing local
/// faster-whisper provider, then return the plaintext transcript.
///
/// Implementation deliberately delegates to
/// [`tardis::transcription::build_provider`] so the same client
/// served by `cargo run -- local-transcribe-file` is reachable from
/// the UI without duplicating the HTTP layer. Provider, model, and
/// language all come from the centralised
/// [`tardis::config::LOCAL_WHISPER_*`] constants inside
/// [`ProviderKind::build`] for `local-whisper`.
///
/// This is **file-based only**: no CPAL microphone capture is
/// started from the UI. The `start_mock_listening` /
/// `stop_mock_listening` commands remain a separate, mock-only
/// surface that does not touch audio hardware.
///
/// Error mapping for the UI hides provider internal wording behind
/// [`normalize_local_transcription_error`] while preserving the
/// original error string in parentheses for diagnosing Docker /
/// format / server issues.
#[tauri::command]
fn transcribe_wav_file_local(file_path: String) -> Result<String, String> {
    let valid_path = validate_wav_path_input(&file_path)?;
    let provider = tardis::transcription::build_provider("local-whisper")
        .map_err(|e| normalize_local_transcription_error(&e.to_string()))?;
    provider
        .transcribe(&valid_path)
        .map_err(|e| normalize_local_transcription_error(&e.to_string()))
}

/// Current wall-clock time as Unix epoch milliseconds.
///
/// The history stores epoch milliseconds rather than a formatted string so
/// the file stays timezone-free and sortable; the UI renders the local time
/// from the same instant.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Append one record to `path`, creating the file (and its directory) on
/// first use.
///
/// Two failure modes matter here, because this is the only write that
/// happens during a live session:
///
/// - A previous crash or a full disk can leave an **unterminated** object at
///   the end of the file. It is dropped before appending, so the new record
///   starts on its own line instead of fusing with the fragment.
/// - If this write itself stops halfway, the bytes it wrote are rolled back.
///   The next append then starts where this one began instead of after a
///   half-written object.
fn append_record_to(path: &std::path::Path, line: &str) -> std::io::Result<()> {
    use std::io::Write as _;

    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)?;
    repair_partial_tail(&mut file)?;
    let start = file.metadata()?.len();
    if let Err(e) = file.write_all(line.as_bytes()) {
        let _ = file.set_len(start);
        return Err(e);
    }
    Ok(())
}

/// Truncate an unterminated trailing record, if there is one.
///
/// Every record `format_record` writes ends in `\n`, so "no trailing
/// newline" is exactly "the last write did not finish". Walks backwards in
/// blocks rather than reading the whole file: history files get long, and
/// this runs on every append.
fn repair_partial_tail(file: &mut std::fs::File) -> std::io::Result<()> {
    use std::io::{Read as _, Seek as _, SeekFrom};

    let mut end = file.metadata()?.len();
    if end == 0 {
        return Ok(());
    }
    let mut block = [0u8; 512];
    while end > 0 {
        let size = end.min(block.len() as u64);
        let start = end - size;
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut block[..size as usize])?;
        if let Some(pos) = block[..size as usize].iter().rposition(|b| *b == b'\n') {
            let keep = start + pos as u64 + 1;
            if keep != file.metadata()?.len() {
                file.set_len(keep)?;
            }
            return Ok(());
        }
        end = start;
    }
    // No newline anywhere: the file is one broken record.
    file.set_len(0)
}

/// Record one stage output in the append-only history file.
///
/// Transcripts and rendered outputs go to the same file, in the order they
/// happened, tagged with `kind` so either can be filtered out.
///
/// Deliberately infallible from the caller's point of view: a read-only
/// working directory, a full disk, or a serialization failure must not take
/// a running session down, so the problem is reported on stderr and the
/// session continues. Empty text is skipped rather than padded into the
/// file, and every other event kind is ignored.
fn append_history(event: &tardis::app::events::AppEvent, path: &std::path::Path) {
    use tardis::app::events::AppEvent;
    use tardis::app::history::{format_record, is_recordable, HistoryRecord};

    let at_ms = now_ms();
    let record = match event {
        AppEvent::Transcript(t) if is_recordable(&t.text) => HistoryRecord::Transcript {
            at_ms,
            text: &t.text,
            duration_ms: t.duration_ms,
            provider: &t.provider,
        },
        AppEvent::Output(o) if is_recordable(&o.text) => HistoryRecord::Output {
            at_ms,
            text: &o.text,
            source_language: &o.source_language,
            target_language: &o.target_language,
            output_kind: o.kind.as_label(),
        },
        _ => return,
    };

    let line = match format_record(&record) {
        Ok(line) => line,
        Err(e) => {
            eprintln!("history: could not format record: {e}");
            return;
        }
    };

    if let Err(e) = append_record_to(path, &line) {
        eprintln!("history: could not append to {}: {e}", path.display());
    }
}

// ===== Crash diagnostics ===================================================

/// `<OS config dir>/tardis/panic.log`, next to `runtime.json`.
pub(crate) fn panic_log_path(base_dir: &Path) -> PathBuf {
    tardis::app::settings_store::settings_file_path(base_dir).with_file_name("panic.log")
}

/// One panic record: when, where, what, and the backtrace.
pub(crate) fn format_panic_record(
    timestamp_secs: u64,
    location: &str,
    payload: &str,
    backtrace: &str,
) -> String {
    format!("=== panic at {timestamp_secs} ({location}) ===\n{payload}\n{backtrace}\n")
}

fn append_record(path: &Path, record: &str) {
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        eprintln!("crash log: could not create {}: {e}", parent.display());
        return;
    }
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        Ok(mut file) => {
            if let Err(e) = writeln!(file, "{record}") {
                eprintln!("crash log: could not write {}: {e}", path.display());
            }
            let _ = file.flush();
        }
        Err(e) => eprintln!("crash log: could not open {}: {e}", path.display()),
    }
}

/// Log every panic (location, message and backtrace) to stderr and to
/// `<config dir>/tardis/panic.log`.
///
/// The shell has no other crash trail: launched from Finder its stderr goes
/// nowhere, so a "the window disappeared" report leaves nothing behind. The
/// backtrace is captured with `Backtrace::force_capture`, which ignores
/// `RUST_BACKTRACE=0`, and the process still exits as it would have. This
/// only observes the panic, it does not swallow it.
fn install_panic_logger(path: PathBuf) {
    let started = timestamp_secs();
    append_record(
        &path,
        &format!(
            "=== shell started at {started} (pid {}) ===",
            std::process::id()
        ),
    );

    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown location>".to_string());
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        };
        let backtrace = format!("{:?}", std::backtrace::Backtrace::force_capture());
        let record = format_panic_record(timestamp_secs(), &location, &payload, &backtrace);
        // Keep the default behaviour on stderr so a terminal launch still
        // shows the panic immediately.
        eprintln!("{record}");
        append_record(&path, &record);
    }));
}

fn timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // Installed before any worker thread starts, so a panic during a
            // live session (the usual suspect) is captured, not lost.
            if let Ok(base_dir) = app.path().config_dir() {
                install_panic_logger(panic_log_path(&base_dir));
            }
            Ok(())
        })
        .manage(Mutex::new(MockAppState::default()))
        // LiveSessionState is registered as `Arc<...>` (not
        // `Mutex<...>`) so the worker thread spawned by
        // `start_live_transcription` can hold its own clone of the
        // `Arc` and reset the inner mutex fields on exit (success,
        // error, or panic) via the `SessionCleanupGuard`
        // RAII guard. Each field already has its own `Mutex`, so
        // an outer lock is not needed.
        //
        // Intentionally nested Arc: Tauri wraps managed state in
        // an internal `Arc<T>` already; explicitly wrapping in
        // another `Arc` here lets the worker thread hold a clone
        // that outlives the command's `tauri::State` borrow. The
        // clone count is also cheaper than threading `&self`
        // through a long-lived worker closure.
        .manage(Arc::new(LiveSessionState::default()))
        .invoke_handler(tauri::generate_handler![
            get_app_status,
            start_mock_listening,
            stop_mock_listening,
            get_mock_transcript,
            get_mock_translation,
            transcribe_wav_file_local,
            start_live_transcription,
            stop_live_transcription,
            get_supported_transcription_providers,
            list_input_devices,
            get_supported_processor_providers,
            get_default_runtime_config,
            load_runtime_settings,
            save_runtime_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running TARDIS UI shell");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_log_sits_next_to_runtime_json() {
        let path = panic_log_path(Path::new("/tmp/fake-config"));
        assert_eq!(
            path,
            PathBuf::from("/tmp/fake-config/tardis/panic.log")
        );
    }

    #[test]
    fn panic_record_carries_time_location_payload_and_backtrace() {
        let record = format_panic_record(
            1_700_000_000,
            "src/lib.rs:42:9",
            "called `Option::unwrap()` on a `None` value",
            "   0: tardis_ui_shell::start_live_transcription",
        );
        assert!(record.starts_with("=== panic at 1700000000 (src/lib.rs:42:9) ==="));
        assert!(record.contains("Option::unwrap"));
        assert!(record.contains("start_live_transcription"));
        assert!(record.ends_with("\n"));
    }

    #[test]
    fn panic_hook_writes_the_backtrace_to_the_log() {
        // Note: the panic hook is process-global, so this test leaves it
        // installed. It only observes panics (the default stderr print is
        // preserved), so other tests are unaffected.
        let dir = std::env::temp_dir().join(format!("tardis-hook-{}", std::process::id()));
        let path = panic_log_path(&dir);
        let _ = std::fs::remove_dir_all(&dir);

        install_panic_logger(path.clone());
        let _ = std::panic::catch_unwind(|| panic!("boom from the test"));

        let contents = std::fs::read_to_string(&path).expect("panic log written");
        assert!(contents.contains("shell started"), "{contents}");
        assert!(contents.contains("boom from the test"), "{contents}");
        assert!(contents.contains("=== panic at"), "{contents}");
        assert!(contents.contains("lib.rs"), "backtrace frames: {contents}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_record_creates_the_log_and_appends() {
        let dir = std::env::temp_dir().join(format!("tardis-panic-{}", std::process::id()));
        let path = panic_log_path(&dir);
        let _ = std::fs::remove_dir_all(&dir);

        append_record(&path, "first");
        append_record(&path, "second");

        let contents = std::fs::read_to_string(&path).expect("log written");
        assert_eq!(contents, "first\nsecond\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_status_is_idle() {
        assert_eq!(MockAppState::default().status(), "Idle");
    }

    #[test]
    fn start_transitions_to_listening() {
        let mut state = MockAppState::default();
        assert_eq!(state.start(), "Listening");
        assert_eq!(state.status(), "Listening");
    }

    #[test]
    fn stop_transitions_to_stopped() {
        let mut state = MockAppState::default();
        state.start();
        assert_eq!(state.stop(), "Stopped");
        assert_eq!(state.status(), "Stopped");
    }

    #[test]
    fn mock_outputs_match_contract() {
        assert_eq!(MOCK_TRANSCRIPT, "mock transcript: speech detected");
        assert_eq!(
            MOCK_TRANSLATION,
            "[mock es] mock transcript: speech detected"
        );
    }

    // ---- transcript history --------------------------------------------

    #[test]
    fn append_record_to_creates_the_directory_and_never_truncates() {
        // A session appends one line per utterance; a truncating writer would
        // silently leave only the last transcript of the day in the file.
        let dir = std::env::temp_dir().join(format!("tardis-history-test-{}", std::process::id()));
        let path = dir.join("nested").join("transcripts.jsonl");
        let _ = std::fs::remove_dir_all(&dir);

        append_record_to(&path, "{\"text\":\"one\"}\n").expect("first append");
        append_record_to(&path, "{\"text\":\"two\"}\n").expect("second append");

        let body = std::fs::read_to_string(&path).expect("read back");
        assert_eq!(
            body.lines().count(),
            2,
            "both appends must survive: {body:?}"
        );
        assert!(body.starts_with("{\"text\":\"one\"}"), "got: {body:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_text_is_skipped_by_the_history() {
        // `append_history` filters on this; an empty provider reply must not
        // add a blank row per silent utterance.
        assert!(!tardis::app::history::is_recordable("  "));
    }

    #[test]
    fn append_record_to_drops_a_partial_tail_left_by_a_crash() {
        // A write that stopped halfway leaves an unterminated object. The
        // next append must not fuse with it: readers that skip malformed
        // lines would otherwise lose that record too.
        let dir = std::env::temp_dir().join(format!(
            "tardis-history-partial-{}",
            std::process::id()
        ));
        let path = dir.join("history.jsonl");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");

        std::fs::write(&path, "{\"text\":\"one\"}\n{\"text\":\"half").expect("seed file");
        append_record_to(&path, "{\"text\":\"two\"}\n").expect("append after a partial write");

        let body = std::fs::read_to_string(&path).expect("read back");
        assert_eq!(body, "{\"text\":\"one\"}\n{\"text\":\"two\"}\n", "got: {body:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_record_to_handles_a_file_with_no_newline_at_all() {
        let dir = std::env::temp_dir().join(format!(
            "tardis-history-nonl-{}",
            std::process::id()
        ));
        let path = dir.join("history.jsonl");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");

        std::fs::write(&path, "{\"text\":\"trunc").expect("seed file");
        append_record_to(&path, "{\"text\":\"two\"}\n").expect("append after a broken file");

        let body = std::fs::read_to_string(&path).expect("read back");
        assert_eq!(body, "{\"text\":\"two\"}\n", "got: {body:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_record_to_keeps_a_complete_file_untouched() {
        // The repair path must not cost bytes when the file is healthy.
        let dir = std::env::temp_dir().join(format!(
            "tardis-history-intact-{}",
            std::process::id()
        ));
        let path = dir.join("history.jsonl");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");

        std::fs::write(&path, "{\"text\":\"one\"}\n").expect("seed file");
        append_record_to(&path, "{\"text\":\"two\"}\n").expect("append");

        let body = std::fs::read_to_string(&path).expect("read back");
        assert_eq!(body, "{\"text\":\"one\"}\n{\"text\":\"two\"}\n", "got: {body:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn history_records_transcripts_and_outputs_in_order_and_ignores_the_rest() {
        use tardis::app::events::{AppEvent, AppStatus, AppTranscriptEvent};
        use tardis::app::live_events::{build_output_event, build_speech_event, build_transcript_event};
        use tardis::processor::processor::{Output, OutputKind};

        let dir = std::env::temp_dir().join(format!("tardis-history-order-{}", std::process::id()));
        let path = dir.join("history.jsonl");
        let _ = std::fs::remove_dir_all(&dir);

        let events = [
            AppEvent::StatusChanged(AppStatus::Listening),
            build_transcript_event(1, "hello".into(), "mock-local".into(), true, 4_320),
            build_output_event(
                1,
                &Output {
                    kind: OutputKind::Translate,
                    source_text: "hello".into(),
                    text: "hola".into(),
                    source_language: "en".into(),
                    target_language: "es".into(),
                    is_final: true,
                    blocked: None,
                },
            ),
            // Status and speech carry no text to record.
            build_speech_event(1, "hola".into(), "es".into(), 900, true),
            AppEvent::StatusChanged(AppStatus::Stopped),
        ];
        for event in &events {
            append_history(event, &path);
        }

        let body = std::fs::read_to_string(&path).expect("read back");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 2, "only transcript + output: {body:?}");
        assert!(
            lines[0].starts_with("{\"kind\":\"transcript\""),
            "got: {}",
            lines[0]
        );
        assert!(
            lines[1].starts_with("{\"kind\":\"output\""),
            "rendered output must be its own record: {}",
            lines[1]
        );

        // An empty provider reply adds nothing: no blank rows per silence.
        let silent = AppEvent::Transcript(AppTranscriptEvent {
            chunk_index: 2,
            text: "   ".into(),
            provider: "local-whisper".into(),
            is_final: true,
            duration_ms: 1_000,
        });
        append_history(&silent, &path);
        let body = std::fs::read_to_string(&path).expect("read back");
        assert_eq!(body.lines().count(), 2, "blank transcript was written: {body:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- validate_wav_path_input --------------------------------------

    #[test]
    fn validate_rejects_empty_string() {
        let err = validate_wav_path_input("").expect_err("empty path must be rejected");
        assert!(
            err.contains("WAV file path"),
            "expected 'WAV file path' hint in error, got: {err}"
        );
    }

    #[test]
    fn validate_rejects_whitespace_only() {
        // Any all-whitespace string (spaces, tabs, newlines) must be
        // rejected the same way as the empty string.
        for input in ["   ", "\t", "\n", " \t\n "] {
            let err = validate_wav_path_input(input)
                .expect_err(&format!("whitespace-only {:?} must be rejected", input));
            assert!(
                err.contains("WAV file path"),
                "expected 'WAV file path' hint in error for input {:?}, got: {err}",
                input
            );
        }
    }

    #[test]
    fn validate_accepts_normal_path() {
        let out =
            validate_wav_path_input("output/chunks/chunk_001.wav").expect("normal path must pass");
        assert_eq!(out, "output/chunks/chunk_001.wav");
    }

    #[test]
    fn validate_trims_surrounding_whitespace() {
        let out = validate_wav_path_input("  output/chunks/chunk_001.wav  ")
            .expect("path with surrounding whitespace must be accepted");
        assert_eq!(out, "output/chunks/chunk_001.wav");
    }

    // ---- normalize_local_transcription_error --------------------------

    #[test]
    fn normalize_file_missing_message_is_user_facing() {
        let raw = "WAV file does not exist: foo.wav";
        let out = normalize_local_transcription_error(raw);
        assert!(
            out.contains("WAV file not found"),
            "expected 'WAV file not found' prefix, got: {out}"
        );
        // Preserves the original detail (path basenamed) so the user
        // can see which file the error refers to.
        assert!(out.contains("foo.wav"), "expected path preservation, got: {out}");
    }

    #[test]
    fn normalize_server_unreachable_message_is_user_facing() {
        let raw = "sending POST request to local faster-whisper server at \
                   http://localhost:8000/v1/audio/transcriptions \
                   — is the Docker container running? \
                   (try: docker compose -f docker/faster-whisper/docker-compose.yml up)";
        let out = normalize_local_transcription_error(raw);
        assert!(
            out.contains("Could not reach"),
            "expected 'Could not reach' prefix, got: {out}"
        );
        // Preserves the original detail (URL + docker hint) so the
        // user can diagnose without opening the Rust logs.
        assert!(
            out.contains("http://localhost:8000"),
            "expected URL preservation, got: {out}"
        );
        assert!(out.contains("docker compose"), "expected docker hint, got: {out}");
    }

    #[test]
    fn normalize_server_uppercase_post_keyword_still_matches() {
        // The provider's actual wording uses upper-case "POST"; the
        // helper must still match because it lowercases internally
        // before keyword-matching. Pass mixed-case input directly so
        // we exercise the case-insensitive path, not a round-trip
        // through `String::to_lowercase`.
        let raw = "Sending POST request to local faster-whisper server at \
                   http://localhost:8000/v1/audio/transcriptions \
                   \u{2014} is the Docker container running?";
        let out = normalize_local_transcription_error(raw);
        assert!(
            out.contains("Could not reach"),
            "expected keyword match despite mixed case, got: {out}"
        );
        // And the original casing must be preserved in the output so
        // the user can still see the URL verbatim.
        assert!(
            out.contains("http://localhost:8000"),
            "expected URL preserved verbatim, got: {out}"
        );
    }

    #[test]
    fn normalize_server_status_message_is_user_facing() {
        let raw = "local faster-whisper server at \
                   http://localhost:8000/v1/audio/transcriptions \
                   returned status 400: bad wav";
        let out = normalize_local_transcription_error(raw);
        assert!(
            out.contains("Server rejected"),
            "expected 'Server rejected' prefix, got: {out}"
        );
        assert!(out.contains("400"), "expected status preservation, got: {out}");
    }

    #[test]
    fn normalize_missing_text_message_is_user_facing() {
        let raw = "response JSON is missing the required 'text' field";
        let out = normalize_local_transcription_error(raw);
        assert!(
            out.contains("transcript text"),
            "expected 'transcript text' phrase, got: {out}"
        );
        assert!(
            out.contains("'text' field"),
            "expected original field name preserved, got: {out}"
        );
    }

    #[test]
    fn normalize_not_a_string_message_is_user_facing() {
        let raw = "response JSON has a 'text' field but it is not a string";
        let out = normalize_local_transcription_error(raw);
        assert!(
            out.contains("Server response format"),
            "expected 'Server response format' prefix, got: {out}"
        );
    }

    #[test]
    fn normalize_unrecognized_error_uses_fallback() {
        let raw = "something completely unexpected";
        let out = normalize_local_transcription_error(raw);
        assert!(
            out.contains("Transcription failed"),
            "expected fallback prefix, got: {out}"
        );
        assert!(
            out.contains("something completely unexpected"),
            "expected original message preserved in fallback, got: {out}"
        );
    }

    #[test]
    fn normalize_preserves_arbitrary_unicode_in_error() {
        // Sanity: the pass-through behavior must not mangle the
        // original detail even when the keyword check matches the
        // fallback branch.
        let raw = "upstream said: \u{1F4E9}";
        let out = normalize_local_transcription_error(raw);
        assert!(out.contains('\u{1F4E9}'), "expected unicode preserved, got: {out}");
    }

    // ---- LiveSessionState: defaults / status flags --------------------

    #[test]
    fn live_session_state_default_is_not_running() {
        let s = LiveSessionState::default();
        assert!(!s.is_session_running(), "default state must not be running");
    }

    #[test]
    fn live_session_state_default_has_no_stop_signal() {
        let s = LiveSessionState::default();
        assert!(
            s.take_stop_signal().is_none(),
            "default state must have no stop signal"
        );
    }

    // ---- LiveSessionState: the Start transition ------------------------

    #[test]
    fn start_accepted_returns_ok_on_fresh_state() {
        let s = LiveSessionState::default();
        assert!(s.apply(SessionInput::StartAccepted).is_ok());
        assert!(s.is_session_running());
        assert_eq!(s.phase(), SessionPhase::Starting);
    }

    #[test]
    fn start_accepted_returns_err_when_already_running() {
        let s = LiveSessionState::default();
        s.apply(SessionInput::StartAccepted)
            .expect("first start must succeed");
        let result = s.apply(SessionInput::StartAccepted);
        assert!(result.is_err(), "second start must fail with Err");
        // The rejected input must not move the phase.
        assert_eq!(s.phase(), SessionPhase::Starting);
    }

    #[test]
    fn start_accepted_works_again_after_worker_exit() {
        // Mirrors the Start → worker-exit → Start lifecycle:
        // after `reset_session`, the next start must succeed.
        let s = LiveSessionState::default();
        s.apply(SessionInput::StartAccepted).unwrap();
        s.reset_session();
        assert!(s.apply(SessionInput::StartAccepted).is_ok());
    }

    #[test]
    fn a_stop_keeps_the_session_slot_until_the_worker_exits() {
        // The documented quick Start → Stop → Start rule: Stop only flips
        // the signal it takes. A second Start while the old worker drains
        // must still see the slot taken, or the new worker could grab the
        // microphone before the old one releases it.
        let s = LiveSessionState::default();
        s.apply(SessionInput::StartAccepted).unwrap();
        let signal = Arc::new(AtomicBool::new(false));
        s.store_stop_signal(Arc::clone(&signal));

        // What `stop_live_transcription` does: take the signal, flip it.
        let taken = s.take_stop_signal().expect("a signal was stored");
        taken.store(true, Ordering::Relaxed);

        assert!(s.is_session_running());
        assert!(s.apply(SessionInput::StartAccepted).is_err());

        s.reset_session(); // the worker finally exits
        assert!(!s.is_session_running());
        assert!(s.apply(SessionInput::StartAccepted).is_ok());
    }

    #[test]
    fn a_poisoned_phase_mutex_recovers_the_phase() {
        // The worker can panic inside the loop; the next Start must not
        // be locked out by a poisoned mutex.
        let state = Arc::new(LiveSessionState::default());
        let poisoner = Arc::clone(&state);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.phase.lock().expect("fresh lock");
            panic!("poison the phase lock");
        })
        .join();

        assert_eq!(state.phase(), SessionPhase::Idle);
        assert!(state.apply(SessionInput::StartAccepted).is_ok());
        assert!(state.is_session_running());
    }

    // ---- LiveSessionState: store / take stop_signal --------------------

    #[test]
    fn store_then_take_stop_signal_round_trip() {
        let s = LiveSessionState::default();
        let signal = Arc::new(AtomicBool::new(false));
        s.store_stop_signal(Arc::clone(&signal));

        let taken = s
            .take_stop_signal()
            .expect("signal must be present after store");
        assert!(Arc::ptr_eq(&taken, &signal));
    }

    #[test]
    fn take_stop_signal_clears_the_field() {
        // After take, the field must be None; otherwise a fast
        // Stop→Start could read a stale signal.
        let s = LiveSessionState::default();
        s.store_stop_signal(Arc::new(AtomicBool::new(false)));
        let _ = s.take_stop_signal();
        assert!(
            s.take_stop_signal().is_none(),
            "take must clear the field"
        );
    }

    #[test]
    fn take_stop_signal_returns_none_when_empty() {
        let s = LiveSessionState::default();
        assert!(s.take_stop_signal().is_none());
    }

    // ---- LiveSessionState: reset_session ------------------------------

    #[test]
    fn reset_session_clears_is_running() {
        let s = LiveSessionState::default();
        s.apply(SessionInput::StartAccepted).unwrap();
        s.reset_session();
        assert!(!s.is_session_running(), "reset must clear is_running");
        assert_eq!(s.phase(), SessionPhase::Idle);
    }

    #[test]
    fn reset_session_clears_stop_signal() {
        let s = LiveSessionState::default();
        s.store_stop_signal(Arc::new(AtomicBool::new(false)));
        s.reset_session();
        assert!(s.take_stop_signal().is_none(), "reset must clear stop_signal");
    }

    // ---- SessionCleanupGuard: Drop semantics --------------------------

    #[test]
    fn cleanup_guard_resets_state_on_drop() {
        // Mirrors the live worker-thread path: allocate state,
        // take the session slot, store a stop signal, drop the
        // guard, state must be back to defaults.
        let state = Arc::new(LiveSessionState::default());
        state.apply(SessionInput::StartAccepted).unwrap();
        state.store_stop_signal(Arc::new(AtomicBool::new(true)));
        assert!(state.is_session_running());
        assert!(state.take_stop_signal().is_some());

        {
            let _guard = SessionCleanupGuard(Arc::clone(&state));
            // While the guard is alive, state is still running.
            assert!(state.is_session_running());
        }

        // After drop, both fields are back to defaults.
        assert!(!state.is_session_running());
        assert_eq!(state.phase(), SessionPhase::Idle);
        assert!(state.take_stop_signal().is_none());
    }

    #[test]
    fn cleanup_guard_runs_even_on_panic() {
        // Panic-safety guarantee: unwinding through the guard
        // must also reset the state. `std::panic::catch_unwind`
        // runs the closure, captures the panic payload, and lets
        // us inspect post-unwind state.
        let state = Arc::new(LiveSessionState::default());
        state.apply(SessionInput::StartAccepted).unwrap();
        state.store_stop_signal(Arc::new(AtomicBool::new(false)));

        let state_for_panic = Arc::clone(&state);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _guard = SessionCleanupGuard(state_for_panic);
            panic!("simulated worker panic");
        }));

        assert!(result.is_err(), "panic must propagate as Err payload");
        assert!(
            !state.is_session_running(),
            "guard's Drop must have released the session slot"
        );
        assert!(
            state.take_stop_signal().is_none(),
            "guard's Drop must have reset stop_signal"
        );
    }

    // ---- worker terminal events ---------------------------------------

    fn ui_payloads(
        events: &[tardis::app::events::AppEvent],
    ) -> Vec<(String, Option<String>, Option<String>)> {
        events
            .iter()
            .map(|event| {
                let ui = tardis::app::ui_events::app_event_to_ui_event(event);
                (ui.kind, ui.status, ui.message)
            })
            .collect()
    }

    #[test]
    fn a_clean_exit_pins_nothing_because_the_runner_already_emits_stopped() {
        // The happy path's single `stopped` comes from the runner; a second
        // one here would duplicate the terminal event the UI unlocks on.
        assert!(worker_exit_events(None).is_empty());
    }

    #[test]
    fn a_failure_pins_error_then_stopped_so_start_unlocks() {
        // Both failure sites funnel through here: a microphone or player
        // that refuses to start, and a stage error mid-session.
        let events = worker_exit_events(Some(
            "Live transcription failed: no input device available".to_string(),
        ));

        // Exactly two events, in this order, in the payload shape the UI
        // already switches on: no new kind, no relabelled status.
        assert_eq!(
            ui_payloads(&events),
            vec![
                (
                    "error".to_string(),
                    None,
                    Some("Live transcription failed: no input device available".to_string()),
                ),
                ("status".to_string(), Some("stopped".to_string()), None),
            ]
        );
    }
}

