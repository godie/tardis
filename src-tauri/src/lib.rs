use std::sync::{Arc, Mutex, MutexGuard};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use tauri::Emitter;

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

/// Holds the stop signal for an active live transcription session.
///
/// When `stop_signal` is `Some`, a background thread is running and
/// can be stopped by setting the flag to `true`. When `None`, no
/// session is active.
#[derive(Debug, Default)]
struct LiveSessionState {
    stop_signal: Option<Arc<AtomicBool>>,
}

fn lock_state<'a>(state: &'a tauri::State<'_, Mutex<MockAppState>>) -> MutexGuard<'a, MockAppState> {
    match state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn lock_session<'a>(
    state: &'a tauri::State<'_, Mutex<LiveSessionState>>,
) -> MutexGuard<'a, LiveSessionState> {
    match state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// ===== New live transcription commands ====================================

/// Start a live transcription session in the background.
///
/// Spawns a thread that captures audio from the default microphone,
/// routes each speech-like chunk through the selected provider, and
/// emits `app-event` Tauri events to the frontend for every
/// [`tardis::app::events::AppEvent`] produced.
///
/// Default provider is `"mock-local"` (no Docker required). Pass
/// `"local-whisper"` to use the self-hosted faster-whisper server.
/// Returns an error if a session is already running.
#[tauri::command]
fn start_live_transcription(
    app: tauri::AppHandle,
    session: tauri::State<'_, Mutex<LiveSessionState>>,
    provider: Option<String>,
) -> Result<String, String> {
    let mut session = lock_session(&session);
    if session.stop_signal.is_some() {
        return Err("A live transcription session is already running. Stop it first.".to_string());
    }

    let provider_name = provider.unwrap_or_else(|| "mock-local".to_string());

    // Validate the provider name early so the user gets a clear
    // error before the background thread starts.
    tardis::transcription::build_provider(&provider_name)
        .map_err(|e| e.to_string())?;

    let stop_signal = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop_signal);
    let app_clone = app.clone();
    let provider_clone = provider_name.clone();

    session.stop_signal = Some(stop_signal);
    drop(session);

    thread::spawn(move || {
        let result = tardis::transcription::live_local::run_live_local_transcription_with_events(
            &provider_clone,
            86400, // long duration — the loop exits via the stop signal
            1000,
            Some(stop_clone),
            move |event| {
                let ui_event =
                    tardis::app::ui_events::app_event_to_ui_event(&event);
                let _ = app_clone.emit("app-event", ui_event);
            },
        );
        if let Err(e) = result {
            let _ = app.emit(
                "app-event",
                tardis::app::ui_events::app_event_to_ui_event(
                    &tardis::app::events::AppEvent::Error(
                        tardis::app::events::AppErrorEvent {
                            message: format!("Live transcription failed: {e}"),
                        },
                    ),
                ),
            );
        }
    });

    Ok(format!("started with provider {provider_name}"))
}

/// Signal the active live transcription session to stop.
///
/// Returns `"stopping"` if a session was active, or an error if
/// no session is running.
#[tauri::command]
fn stop_live_transcription(
    session: tauri::State<'_, Mutex<LiveSessionState>>,
) -> Result<String, String> {
    let mut session = lock_session(&session);
    match session.stop_signal.take() {
        Some(signal) => {
            signal.store(true, Ordering::Relaxed);
            Ok("stopping".to_string())
        }
        None => Err("No live transcription session is running.".to_string()),
    }
}

/// Return the list of supported transcription provider names.
///
/// Pure — no I/O, no state access. Used by the frontend to
/// populate the provider selector.
#[tauri::command]
fn get_supported_providers() -> Vec<String> {
    vec!["mock-local".to_string(), "local-whisper".to_string()]
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
/// check whether the file actually exists on disk — the provider's
/// own `path.is_file()` check surfaces the file-not-found case with
/// a more specific message ("WAV file does not exist: <path>").
///
/// Pure — no FS or HTTP access.
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
/// Pure — operates on a string. Matching is case-insensitive.
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
/// This is **file-based only** — no CPAL microphone capture is
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Mutex::new(MockAppState::default()))
        .manage(Mutex::new(LiveSessionState::default()))
        .invoke_handler(tauri::generate_handler![
            get_app_status,
            start_mock_listening,
            stop_mock_listening,
            get_mock_transcript,
            get_mock_translation,
            transcribe_wav_file_local,
            start_live_transcription,
            stop_live_transcription,
            get_supported_providers,
        ])
        .run(tauri::generate_context!())
        .expect("error while running TARDIS UI shell");
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

