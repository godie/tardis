const transcriptCopy = "mock transcript: speech detected";
const translationCopy = "[mock es] mock transcript: speech detected";

const statusIndicator = document.querySelector("#status-indicator");
const transcriptPanel = document.querySelector("#transcript-content");
const translationPanel = document.querySelector("#translation-content");
const startButton = document.querySelector("#start-button");
const stopButton = document.querySelector("#stop-button");
const targetLanguage = document.querySelector("#target-language");

// Local WAV transcription controls.
const wavFilePathInput = document.querySelector("#wav-file-path");
const transcribeFileButton = document.querySelector("#transcribe-file-button");
const fileTranscribeStatus = document.querySelector("#file-transcribe-status");
const fileTranscribeOutput = document.querySelector("#file-transcribe-output");
const fileTranscribeError = document.querySelector("#file-transcribe-error");

function hasTauriInvoke() {
  return Boolean(window.__TAURI__?.core?.invoke);
}

async function invoke(command) {
  if (!hasTauriInvoke()) {
    throw new Error("Tauri invoke API is unavailable. Start this screen inside the Tauri shell.");
  }
  return window.__TAURI__.core.invoke(command);
}

function setStatus(status) {
  statusIndicator.textContent = status;
  statusIndicator.dataset.status = status;
}

function setPanels(transcript, translation) {
  transcriptPanel.textContent = transcript;
  transcriptPanel.classList.remove("muted");
  translationPanel.textContent = translation;
  translationPanel.classList.remove("muted");
}

async function syncInitialState() {
  if (!hasTauriInvoke()) {
    setStatus("Idle");
    transcriptPanel.textContent = "Run the Tauri shell to activate the mock backend commands.";
    translationPanel.textContent = "The static preview is loaded, but invoke() is unavailable outside Tauri.";
    return;
  }

  const status = await invoke("get_app_status");
  setStatus(status);
}

startButton.addEventListener("click", async () => {
  const status = await invoke("start_mock_listening");
  setStatus(status);

  const transcript = await invoke("get_mock_transcript");
  let translation = await invoke("get_mock_translation");
  const selectedTarget = targetLanguage.value;

  if (selectedTarget !== "es") {
    translation = `[mock ${selectedTarget}] ${transcript}`;
  }

  setPanels(transcript, translation);
});

stopButton.addEventListener("click", async () => {
  const status = await invoke("stop_mock_listening");
  setStatus(status);
});

targetLanguage.addEventListener("change", () => {
  if (transcriptPanel.textContent !== transcriptCopy) {
    return;
  }

  translationPanel.textContent = `[mock ${targetLanguage.value}] ${transcriptCopy}`;
});

// ===== Local WAV transcription ============================================
//
// File-based only — no CPAL stream is started. Calls the
// `transcribe_wav_file_local` Rust command, which delegates to
// `tardis::transcription::build_provider("local-whisper")` and
// returns plaintext (or a user-facing error string). Errors are
// already shaped by `normalize_local_transcription_error` on the
// Rust side; here we only need to surface them.

function setFileTranscribeStatus(message, state) {
  fileTranscribeStatus.textContent = message;
  fileTranscribeStatus.dataset.state = state;
}

function setFileTranscribeError(message) {
  if (!message) {
    fileTranscribeError.hidden = true;
    fileTranscribeError.textContent = "";
    return;
  }
  fileTranscribeError.hidden = false;
  fileTranscribeError.textContent = message;
}

function setFileTranscribeOutput(message, isMuted) {
  fileTranscribeOutput.textContent = message;
  fileTranscribeOutput.classList.toggle("muted", Boolean(isMuted));
}

async function runFileTranscribe() {
  const filePath = wavFilePathInput.value;

  setFileTranscribeError("");
  setFileTranscribeStatus("Transcribing\u2026", "busy");
  setFileTranscribeOutput("Transcribing\u2026", true);
  transcribeFileButton.disabled = true;
  transcribeFileButton.dataset.state = "busy";

  try {
    if (!hasTauriInvoke()) {
      throw new Error(
        "Tauri invoke API is unavailable. Start this screen inside the Tauri shell."
      );
    }
    const transcript = await invoke("transcribe_wav_file_local", { filePath });
    setFileTranscribeStatus(`Done \u2014 transcribed ${filePath}`, "done");
    setFileTranscribeOutput(transcript || "(empty transcript)", false);
  } catch (error) {
    const message = String(error);
    setFileTranscribeStatus(`Failed \u2014 could not transcribe ${filePath}`, "error");
    setFileTranscribeOutput("(no transcript)", true);
    setFileTranscribeError(message);
  } finally {
    transcribeFileButton.disabled = false;
    transcribeFileButton.dataset.state = "idle";
  }
}

transcribeFileButton.addEventListener("click", runFileTranscribe);

wavFilePathInput.addEventListener("keydown", (event) => {
  // Enter inside the path input triggers the same flow as clicking
  // the button so the user does not have to leave the keyboard to
  // retry after fixing a typo.
  if (event.key === "Enter") {
    event.preventDefault();
    runFileTranscribe();
  }
});

window.addEventListener("DOMContentLoaded", () => {
  syncInitialState().catch((error) => {
    setStatus("Stopped");
    transcriptPanel.textContent = "Unable to reach the mock backend commands.";
    translationPanel.textContent = String(error);
  });
});

