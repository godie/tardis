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
const sourceLanguage = document.querySelector("#source-language");
const providerSelect = document.querySelector("#provider-select");
const chunkDurationMsInput = document.querySelector("#chunk-duration-ms");
const volumeThresholdInput = document.querySelector("#volume-threshold");

// All session settings live here so Start/Stop, the backend
// `app-event` listener, and the load-from-backend populator can
// flip `disabled` in lockstep. Order matches `tardis::app::
// config::AppRuntimeConfig` so JSON serialisation keeps the
// field names a 1:1 cross-check for serde.
const SESSION_SETTINGS_INPUTS = [
  providerSelect,
  sourceLanguage,
  targetLanguage,
  chunkDurationMsInput,
  volumeThresholdInput,
];

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
if (sourceLanguage) {
  sourceLanguage.addEventListener("change", updateLanguageHint);
}

async function startLiveTranscription() {
  if (isListening) return;

  setStatus("Starting\u2026");
  setLiveError("");

  // Build a single AppRuntimeConfig-shaped object from the
  // settings panel. The backend re-validates; client-side Number()
  // coercion plus a "raw string" fallback keeps the page alive
  // even if the user has not yet touched an input (the
  // server-side default applies).
  const config = {
    transcription_provider: providerSelect.value,
    source_language: sourceLanguage.value,
    target_language: targetLanguage.value,
    chunk_duration_ms: Number(chunkDurationMsInput.value),
    volume_threshold: Number(volumeThresholdInput.value),
  };

  try {
    const msg = await invoke("start_live_transcription", { config });
    log("start_live_transcription:", msg, "config:", config);
    isListening = true;
    startButton.disabled = true;
    startButton.dataset.state = "busy";
    stopButton.disabled = false;
    // Lock the settings panel while a session is in progress so
    // an in-flight live runner is not silently reconfig'd. The
    // backend ignores mid-session changes anyway.
    disableSessionSettings();
  } catch (error) {
    setStatus("Error");
    setLiveError(`Failed to start: ${error}`);
    isListening = false;
  }
}

function disableSessionSettings() {
  for (const el of SESSION_SETTINGS_INPUTS) {
    if (el) el.disabled = true;
  }
}

function enableSessionSettings() {
  for (const el of SESSION_SETTINGS_INPUTS) {
    if (el) el.disabled = false;
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
            // The backend will produce a `stopped` event when the
            // worker exits, so settings re-enable happens there.
            // We deliberately do not call disableSessionSettings()
            // here because Start may already have done it.
          } else if (status === "stopped") {
            isListening = false;
            startButton.disabled = false;
            startButton.dataset.state = "idle";
            stopButton.disabled = true;
            // Worker has fully exited; let the user adjust
            // settings before pressing Start again.
            enableSessionSettings();
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
  await loadFromBackendSettings();
  updateLanguageHint();
}

// Populate the settings inputs from the Tauri backend so the UI
// matches `tardis::app::config::AppRuntimeConfig::default()` and
// the official provider list rather than hardcoded HTML values.
// Falls back gracefully if either invoke is unavailable (e.g.
// the static-preview environment outside the shell).
async function loadFromBackendSettings() {
  // 1. Try the persisted config first; if the file is missing or
  //    corrupt, fall back to canonical defaults so first-run UX
  //    matches the getting-started docs exactly. The backend
  //    returns the same JSON shape as `AppRuntimeConfig`
  //    (serde round-tripped).
  let defaults = null;
  try {
    defaults = await invoke("load_runtime_settings");
    log("loaded persisted settings:", defaults);
  } catch (loadError) {
    log("could not load persisted settings (using defaults):", loadError);
    try {
      defaults = await invoke("get_default_runtime_config");
      log("loaded default runtime config:", defaults);
    } catch (defaultError) {
      log("could not load default runtime config:", defaultError);
    }
  }

  if (defaults && typeof defaults === "object") {
    if (typeof defaults.transcription_provider === "string") {
      providerSelect.value = defaults.transcription_provider;
    }
    if (typeof defaults.source_language === "string") {
      sourceLanguage.value = defaults.source_language;
    }
    if (typeof defaults.target_language === "string") {
      targetLanguage.value = defaults.target_language;
    }
    if (Number.isFinite(defaults.chunk_duration_ms)) {
      chunkDurationMsInput.value = String(defaults.chunk_duration_ms);
    }
    if (Number.isFinite(defaults.volume_threshold)) {
      volumeThresholdInput.value = String(defaults.volume_threshold);
    }
  }

  try {
    const providers = await invoke("get_supported_transcription_providers");
    if (Array.isArray(providers) && providers.length > 0) {
      // Rebuild the provider <select> from the backend list so a
      // future provider shows up without touching the HTML.
      providerSelect.innerHTML = "";
      for (const name of providers) {
        const opt = document.createElement("option");
        opt.value = name;
        opt.textContent = name;
        providerSelect.appendChild(opt);
      }
      log("loaded supported providers:", providers);
    }
  } catch (error) {
    log("could not load supported providers:", error);
  }

  // 2. Auto-save on every committed change so settings persist
  //    across Tauri shell restarts. The `change` event fires
  //    on <select>/<input type=number> when the user commits a
  //    new value (option pick, blur, or Enter). We deliberately
  //    avoid `input` here so intermediate keystrokes that
  //    produce out-of-bounds values don't bounce a rejected
  //    config back and forth to disk.
  for (const input of SESSION_SETTINGS_INPUTS) {
    if (!input) continue;
    input.addEventListener("change", saveCurrentSettings);
  }
}

// Snapshot the current settings panel into a config and push it
// to the backend for atomic persistence. Backend re-validates
// defensively; client-side clobber-by-blur-step is rare enough
// that we can simply log the rejection and leave the inputs as
// the user set them (the next valid change re-saves).
async function saveCurrentSettings() {
  const config = {
    transcription_provider: providerSelect.value,
    source_language: sourceLanguage.value,
    target_language: targetLanguage.value,
    chunk_duration_ms: Number(chunkDurationMsInput.value),
    volume_threshold: Number(volumeThresholdInput.value),
  };
  try {
    await invoke("save_runtime_settings", { config });
    log("saved runtime settings:", config);
  } catch (error) {
    // Don't block the UI on save failures — the user can still
    // press Start Listening, and the backend will re-validate
    // using the active config object the frontend sends in that
    // call.
    log("could not save runtime settings:", error);
  }
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
