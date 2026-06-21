const transcriptCopy = "mock transcript: speech detected";
const translationCopy = "[mock es] mock transcript: speech detected";

const statusIndicator = document.querySelector("#status-indicator");
const transcriptPanel = document.querySelector("#transcript-content");
const translationPanel = document.querySelector("#translation-content");
const startButton = document.querySelector("#start-button");
const stopButton = document.querySelector("#stop-button");
const targetLanguage = document.querySelector("#target-language");

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

window.addEventListener("DOMContentLoaded", () => {
  syncInitialState().catch((error) => {
    setStatus("Stopped");
    transcriptPanel.textContent = "Unable to reach the mock backend commands.";
    translationPanel.textContent = String(error);
  });
});

