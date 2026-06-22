'use strict';

const transcriptCopy = "mock transcript: speech detected";
const translationCopy = "[mock es] mock transcript: speech detected";

const statusIndicator = document.querySelector("#status-indicator");
const transcriptPanel = document.querySelector("#transcript-content");
const translationPanel = document.querySelector("#translation-content");
const liveError = document.querySelector("#live-error");
const startButton = document.querySelector("#start-button");
const stopButton = document.querySelector("#stop-button");
const targetLanguage = document.querySelector("#target-language");
const providerSelect = document.querySelector("#provider-select");

const DEV = true; // Set to false before release to suppress console logs
const providerHint = document.querySelector("#provider-hint");

// Local WAV transcription controls.
const wavFilePathInput = document.querySelector("#wav-file-path");
const transcribeFileButton = document.querySelector("#transcribe-file-button");
const fileTranscribeStatus = document.querySelector("#file-transcribe-status");
const fileTranscribeOutput = document.querySelector("#file-transcribe-output");
const fileTranscribeError = document.querySelector("#file-transcribe-error");

function hasTauriInvoke() {
  return Boolean(window.__TAURI__?.core?.invoke);
}

function log(...args) {
  if (DEV) console.log(...args);
}

async function invoke(command, args) {
  if (!hasTauriInvoke()) {
    throw new Error("Tauri invoke API is unavailable. Start this screen inside the Tauri shell.");
  }
  return window.__TAURI__.core.invoke(command, args);
}

function setStatus(status) {
  statusIndicator.textContent = status;
  statusIndicator.dataset.status = status;
  statusIndicator.setAttribute("aria-label", `Current status: ${status}`);
}

function setLiveError(message) {
  if (!message) {
    liveError.hidden = true;
    liveError.textContent = "";
    return;
  }
  liveError.hidden = false;
  liveError.textContent = message;
}

function setPanels(transcript, translation) {
  transcriptPanel.textContent = transcript;
  transcriptPanel.classList.remove("muted");
  translationPanel.textContent = translation;
  translationPanel.classList.remove("muted");
}

function appendTranscript(text) {
  transcriptPanel.textContent = text;
  transcriptPanel.classList.remove("muted");
}

function appendTranslation(text) {
  translationPanel.textContent = text;
  translationPanel.classList.remove("muted");
}

// ===== Live transcription via Tauri events ================================

let isListening = false;

const languageHint = document.querySelector("#language-hint");

function updateLanguageHint() {
  const source = document.querySelector("#source-language").value;
  const target = document.querySelector("#target-language").value;
  if (languageHint) {
    languageHint.textContent = `During live transcription, translation uses ${source} \u2192 ${target}. The mock translator produces deterministic placeholder output.`;
  }
}

targetLanguage.addEventListener("change", updateLanguageHint);
const sourceLanguage = document.querySelector("#source-language");
if (sourceLanguage) {
  sourceLanguage.addEventListener("change", updateLanguageHint);
}

async function startLiveTranscription() {
  if (isListening) return;

  setStatus("Starting\u2026");
  setLiveError("");

  try {
    const provider = providerSelect.value;
    const msg = await invoke("start_live_transcription", { provider });
    log("start_live_transcription:", msg);
    isListening = true;
    startButton.disabled = true;
    startButton.dataset.state = "busy";
    stopButton.disabled = false;
  } catch (error) {
    setStatus("Error");
    setLiveError(`Failed to start: ${error}`);
    isListening = false;
  }
}

async function stopLiveTranscription() {
  if (!isListening) return;

  try {
    const msg = await invoke("stop_live_transcription");
    log("stop_live_transcription:", msg);
    isListening = false;
    startButton.disabled = false;
    startButton.dataset.state = "idle";
    stopButton.disabled = true;
  } catch (error) {
    log("stop error:", error);
  }
}

// Listen for Tauri events emitted by the backend.
function setupEventListener() {
  if (!window.__TAURI__?.event?.listen) {
    console.warn("Tauri event API unavailable. Live transcription events will not be received.");
    return;
  }

  window.__TAURI__.event.listen("app-event", (event) => {
    const payload = event.payload;
    log("app-event:", payload);

    switch (payload.kind) {
      case "status": {
        const status = payload.status;
        if (status) {
          setStatus(status.charAt(0).toUpperCase() + status.slice(1));
          if (status === "listening") {
            isListening = true;
            startButton.disabled = true;
            startButton.dataset.state = "busy";
            stopButton.disabled = false;
          } else if (status === "stopped") {
            isListening = false;
            startButton.disabled = false;
            startButton.dataset.state = "idle";
            stopButton.disabled = true;
          }
        }
        break;
      }
      case "transcript": {
        if (payload.text) {
          appendTranscript(payload.text);
          setLiveError("");
        }
        break;
      }
      case "translation": {
        if (payload.translated_text) {
          appendTranslation(payload.translated_text);
        }
        break;
      }
      case "error": {
        if (payload.message) {
          setLiveError(payload.message);
        }
        break;
      }
    }
  });
}

// ===== Existing mock / init flow ===========================================

async function syncInitialState() {
  if (!hasTauriInvoke()) {
    setStatus("Idle");
    transcriptPanel.textContent = "Run the Tauri shell to activate the mock backend commands.";
    translationPanel.textContent = "The static preview is loaded, but invoke() is unavailable outside Tauri.";
    return;
  }

  const status = await invoke("get_app_status");
  setStatus(status);
  setupEventListener();
}

startButton.addEventListener("click", async () => {
  await startLiveTranscription();
});

stopButton.addEventListener("click", async () => {
  await stopLiveTranscription();
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
