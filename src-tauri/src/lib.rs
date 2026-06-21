use std::sync::{Mutex, MutexGuard};

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

fn lock_state<'a>(state: &'a tauri::State<'_, Mutex<MockAppState>>) -> MutexGuard<'a, MockAppState> {
    match state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Mutex::new(MockAppState::default()))
        .invoke_handler(tauri::generate_handler![
            get_app_status,
            start_mock_listening,
            stop_mock_listening,
            get_mock_transcript,
            get_mock_translation
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
}

