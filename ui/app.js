'use strict';

const transcriptCopy = "mock transcript: speech detected";
const translationCopy = "[mock es] mock transcript: speech detected";

const statusIndicator = document.querySelector("#status-indicator");
const statusAnnouncer = document.querySelector("#status-announcer");
const transcriptPanel = document.querySelector("#transcript-content");
const transcriptEmpty = document.querySelector("#transcript-empty");
const translationPanel = document.querySelector("#translation-content");
const translationEmpty = document.querySelector("#translation-empty");
const speechStatus = document.querySelector("#speech-status");
const liveError = document.querySelector("#live-error");
const startButton = document.querySelector("#start-button");
const stopButton = document.querySelector("#stop-button");
const targetLanguage = document.querySelector("#target-language");
const sourceLanguage = document.querySelector("#source-language");
const providerSelect = document.querySelector("#provider-select");
const processorSelect = document.querySelector("#processor-select");
const modeSelect = document.querySelector("#mode-select");
const hostNamesInput = document.querySelector("#host-names");
const inputDeviceSelect = document.querySelector("#input-device");
const chunkDurationMsInput = document.querySelector("#chunk-duration-ms");
const volumeThresholdInput = document.querySelector("#volume-threshold");
const hangoverMsInput = document.querySelector("#hangover-ms");
const minSpeechMsInput = document.querySelector("#min-speech-ms");
const maxUtteranceMsInput = document.querySelector("#max-utterance-ms");

// All session settings live here so Start/Stop, the backend
// `app-event` listener, and the load-from-backend populator can
// flip `disabled` in lockstep. Order matches `tardis::app::
// config::AppRuntimeConfig` so JSON serialisation keeps the
// field names a 1:1 cross-check for serde.
const SESSION_SETTINGS_INPUTS = [
  providerSelect,
  processorSelect,
  modeSelect,
  hostNamesInput,
  inputDeviceSelect,
  sourceLanguage,
  targetLanguage,
  chunkDurationMsInput,
  volumeThresholdInput,
  hangoverMsInput,
  minSpeechMsInput,
  maxUtteranceMsInput,
];

const DEV = true; // Set to false before release to suppress console logs

// Rows kept per history panel. The whole session is still on disk in
// output/history.jsonl; the panels only need the recent tail, and an
// unbounded list keeps every utterance in the webview's DOM for the rest of
// the session.
const MAX_HISTORY_ROWS = 200;

// The persisted capture source. The panel has no source control yet (the
// CLI and `runtime.json` select it), but Start and Save must send the loaded
// value back: omitting it lets serde's default silently turn a system
// session into a microphone session.
let currentAudioSource = "mic";

// Append one row to a history panel and retire the oldest rows past the cap,
// keeping the newest row in view.
function appendHistoryItem(panel, item) {
  panel.append(item);
  while (panel.children.length > MAX_HISTORY_ROWS) {
    panel.firstElementChild.remove();
  }
  panel.scrollTop = panel.scrollHeight;
}
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
  // The pill itself is not a live region, so a status change would
  // otherwise be silent for screen reader users.
  if (statusAnnouncer) statusAnnouncer.textContent = `Status: ${status}`;
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

// One row per transcribed utterance. The panel used to overwrite its text,
// so only the newest sentence survived; the list keeps the whole session and
// each row carries the two facts that make a row readable later: when it was
// heard, and how much audio it came from.
function appendTranscript(text, durationMs, provider) {
  if (!text) return;
  if (transcriptEmpty) transcriptEmpty.hidden = true;

  const now = new Date();
  const item = document.createElement("li");
  item.className = "history-item";

  const time = document.createElement("time");
  time.className = "history-time";
  time.dateTime = now.toISOString();
  time.textContent = now.toLocaleTimeString();

  const body = document.createElement("span");
  body.className = "history-text";
  body.dir = "auto";
  body.textContent = text;

  const meta = document.createElement("span");
  meta.className = "history-meta";
  const parts = [];
  if (Number.isFinite(Number(durationMs)) && Number(durationMs) > 0) {
    parts.push(`${(Number(durationMs) / 1000).toFixed(1)} s`);
  }
  if (provider) parts.push(provider);
  meta.textContent = parts.join(" \u00b7 ");

  item.append(time, body, meta);
  // Newest last: the helper keeps the newest row in view as the session
  // runs, and drops rows past MAX_HISTORY_ROWS from the top.
  appendHistoryItem(transcriptPanel, item);
}

// Same shape as the transcript history: one row per rendered output, so the
// two panels line up and a reader can see which language pair produced each
// line. The text is what the process stage returned, not the transcript.
function appendTranslation(text, sourceLang, targetLang, blocked) {
  if (!text) return;
  if (translationEmpty) translationEmpty.hidden = true;

  const now = new Date();
  const item = document.createElement("li");
  item.className = "history-item";

  const time = document.createElement("time");
  time.className = "history-time";
  time.dateTime = now.toISOString();
  time.textContent = now.toLocaleTimeString();

  const body = document.createElement("span");
  body.className = "history-text";
  body.dir = "auto";
  body.textContent = text;

  const meta = document.createElement("span");
  meta.className = "history-meta";
  const pair = sourceLang && targetLang ? `${sourceLang} \u2192 ${targetLang}` : "";
  meta.textContent = pair;

  item.append(time, body, meta);

  // A guard block is a visible "silent in the room" marker on the
  // entry itself: the reason and the never-spoken fact belong
  // together.
  if (blocked) {
    const blockedBadge = document.createElement("strong");
    blockedBadge.className = "blocked-badge";
    blockedBadge.textContent = `blocked (${blocked})`;
    item.append(blockedBadge);
  }

  appendHistoryItem(translationPanel, item);
}

// The speech line reports what the backend rendered and asked the
// player to speak. `duration_ms` comes from the same pure helper the
// CLI prints, so both surfaces agree to the millisecond.
function showSpokenAudio(text, language, durationMs) {
  if (!speechStatus || !text) return;
  const seconds = (Number(durationMs) / 1000).toFixed(1);
  speechStatus.textContent = `Spoken · ${seconds} s · ${language} — ${text}`;
  speechStatus.dataset.state = "spoken";
  speechStatus.hidden = false;
}

function clearSpokenAudio() {
  if (!speechStatus) return;
  speechStatus.hidden = true;
  speechStatus.textContent = "Spoken audio appears here once a chunk is synthesized.";
  delete speechStatus.dataset.state;
}

// ===== Live transcription via Tauri events ================================

// The last session phase the backend reported: `status` events carry it
// (`UiAppEvent::phase`, computed by `SessionPhase::apply_status`) and
// `get_session_phase` answers it on boot and after a command whose outcome
// the UI cannot infer. `src/app/session_phase.rs` owns the transition
// table; this file draws the phase it was handed instead of keeping a
// second copy of the rules.
let sessionPhase = "idle";

// Draw the Start/Stop/settings layout for one phase. Idle is the only
// phase with an enabled Start and an unlocked panel. Stop is live only
// while capture runs: during `stopping` a retry would race the worker's
// teardown, and during `starting` there is nothing to stop yet.
function renderSessionPhase(phase) {
  sessionPhase = phase;
  const idle = phase === "idle";
  startButton.disabled = !idle;
  startButton.dataset.state = idle ? "idle" : "busy";
  stopButton.disabled = phase !== "listening";
  if (idle) {
    enableSessionSettings();
  } else {
    disableSessionSettings();
  }
}

// Ask the backend which phase its session slot holds, and draw it.
//
// The local copy goes stale when an event outruns an invoke: a terminal
// status during the start flight, or a Stop the backend refuses because
// the worker already exited. Only the backend can say where that left the
// session.
async function resyncSessionPhase() {
  try {
    const phase = await invoke("get_session_phase");
    if (typeof phase === "string") {
      renderSessionPhase(phase);
    }
  } catch (error) {
    log("could not read the session phase:", error);
  }
}

const languageHint = document.querySelector("#language-hint");

function updateLanguageHint() {
  const source = document.querySelector("#source-language").value;
  const target = document.querySelector("#target-language").value;
  if (languageHint) {
    languageHint.textContent = `During live transcription, the process stage renders output in ${target} from ${source}. The mock processor produces deterministic placeholder output.`;
  }
}

targetLanguage.addEventListener("change", updateLanguageHint);
if (sourceLanguage) {
  sourceLanguage.addEventListener("change", updateLanguageHint);
}

// The host names input only matters in respond mode; in translate mode
// it is disabled so Start cannot silently pair host names with a mode
// that ignores them.
function syncHostNamesField() {
  if (hostNamesInput) {
    hostNamesInput.disabled = modeSelect.value !== "respond";
  }
}
if (modeSelect) {
  modeSelect.addEventListener("change", syncHostNamesField);
  syncHostNamesField();
}

// Read a numeric field the way the backend will see it. A cleared box, a
// locale decimal comma typed into <input type="number">, or a pasted run
// of digits out of range all sanitize to "" — and Number("") is 0, which
// would quietly start a session with a zero chunk window. Reject instead
// of guessing, and mark the field so the problem is visible.
//
// `integer` mirrors the backend type: `chunk_duration_ms` deserializes as
// a Rust `u64`, so 250.5 passes a range check and then fails to
// deserialize the whole config. The step attribute does not catch it:
// `valueAsNumber` keeps the fraction.
function readNumberInput(input, label, { integer = false } = {}) {
  const min = Number(input.min);
  const max = Number(input.max);
  const value = input.valueAsNumber;
  const outOfRange =
    input.value.trim() === "" || !Number.isFinite(value) || value < min || value > max;
  if (outOfRange || (integer && !Number.isInteger(value))) {
    input.setAttribute("aria-invalid", "true");
    const decimals = input.step.includes(".") ? " (use a dot for decimals)" : "";
    const kind = integer ? "a whole number" : "a number";
    return {
      ok: false,
      message: `${label} must be ${kind} from ${min} to ${max}${decimals}.`,
    };
  }
  input.removeAttribute("aria-invalid");
  return { ok: true, value };
}

// The endpointing pair has one cross-field rule the Backend also enforces:
// a cap below the minimum would flush every burst before it could qualify,
// so the session would transcribe nothing. Check it before starting instead
// of letting the backend rejection look like a random failure.
function readEndpointInputs() {
  const hangover = readNumberInput(hangoverMsInput, "Pause before closing", { integer: true });
  const minSpeech = readNumberInput(minSpeechMsInput, "Minimum speech", { integer: true });
  const maxUtterance = readNumberInput(maxUtteranceMsInput, "Maximum utterance", {
    integer: true,
  });
  const firstBad = [hangover, minSpeech, maxUtterance].find((result) => !result.ok);
  if (firstBad) return firstBad;
  if (maxUtterance.value <= 0 || maxUtterance.value < minSpeech.value) {
    maxUtteranceMsInput.setAttribute("aria-invalid", "true");
    return {
      ok: false,
      message: "Maximum utterance must be greater than 0 and at least the minimum speech length.",
    };
  }
  maxUtteranceMsInput.removeAttribute("aria-invalid");
  return {
    ok: true,
    value: maxUtterance.value,
    hangover_ms: hangover.value,
    min_speech_ms: minSpeech.value,
    max_utterance_ms: maxUtterance.value,
  };
}

let startInFlight = false;

async function startLiveTranscription() {
  // A second click while the first invoke is still in flight would
  // spawn two backend runners. The guard is request-aware, not
  // phase-only: a backend event landing before the invoke resolves
  // (a stale `stopped`, for example) must not re-open Start while the
  // first request is unresolved.
  if (sessionPhase !== "idle" || startInFlight) return;

  const chunk = readNumberInput(chunkDurationMsInput, "Chunk duration", { integer: true });
  const threshold = readNumberInput(volumeThresholdInput, "Volume threshold");
  const endpointing = readEndpointInputs();
  const problem = [chunk, threshold, endpointing].find((result) => !result.ok);
  if (problem) {
    setLiveError(problem.message);
    const firstInvalid = document.querySelector('[aria-invalid="true"]');
    if (firstInvalid) firstInvalid.focus();
    return;
  }

  startInFlight = true;
  // The click is `StartAccepted`: the backend takes the session slot
  // before it answers, so the panel locks now and not when the invoke
  // settles.
  renderSessionPhase("starting");
  setStatus("Starting\u2026");
  setLiveError("");
  // A new session starts with no spoken line: the previous run's
  // duration would otherwise look like it belongs to this session.
  clearSpokenAudio();

  // Build a single AppRuntimeConfig-shaped object from the settings panel.
  const config = {
    transcription_provider: providerSelect.value,
    processor_provider: processorSelect.value,
    mode: buildAppMode(),
    source_language: sourceLanguage.value,
    target_language: targetLanguage.value,
    chunk_duration_ms: chunk.value,
    volume_threshold: threshold.value,
    hangover_ms: endpointing.hangover_ms,
    min_speech_ms: endpointing.min_speech_ms,
    max_utterance_ms: endpointing.max_utterance_ms,
    // `""` is the "System default" option; the backend reads `null` as
    // "let the OS choose".
    input_device: inputDeviceSelect.value || null,
    audio_source: currentAudioSource,
  };

  try {
    const msg = await invoke("start_live_transcription", { config });
    log("start_live_transcription:", msg, "config:", config);
    // Only the settled invoke owns the transition out of `starting`, and
    // only while the phase is still there: a terminal event that landed
    // during the flight already ended the session (a failed worker, or an
    // immediate Stop) and must not be revived here.
    if (sessionPhase === "starting") {
      renderSessionPhase("listening");
    }
  } catch (error) {
    setStatus("Error");
    setLiveError(`Failed to start: ${error}`);
    // `StartFailed` out of `starting` lands in `idle` (the table in
    // `src/app/session_phase.rs`): the backend refused, so this page has
    // no session and the panel must not stay locked on one.
    renderSessionPhase("idle");
  } finally {
    startInFlight = false;
  }
}

// `AppMode` as the backend's serde expects: a `kind` tag, plus the
// respond policy only when the mode select says respond. Host names
// split on commas; blanks drop out, the rest go through as typed (the
// backend normalizes and validates them).
function buildAppMode() {
  if (modeSelect.value !== "respond") {
    return { kind: "translate" };
  }
  const hostNames = hostNamesInput.value
    .split(",")
    .map((name) => name.trim())
    .filter((name) => name.length > 0);
  return {
    kind: "respond",
    policy: {
      host_names: hostNames,
      // v1 ships the conservative defaults; validation rejects anything
      // looser, so the panel does not offer the loosened form.
      answer_questions_only: true,
      require_name: true,
      allow_commitments: false,
    },
  };
}

function disableSessionSettings() {  for (const el of SESSION_SETTINGS_INPUTS) {
    if (el) el.disabled = true;
  }
}

function enableSessionSettings() {
  for (const el of SESSION_SETTINGS_INPUTS) {
    if (el) el.disabled = false;
  }
}

// One Stop at a time. The phase does not move until the backend accepts
// the stop, so the button stays live for the whole round trip (a click
// that lands mid-flight must not send a second request).
let stopInFlight = false;

async function stopLiveTranscription() {
  if (sessionPhase !== "listening" || stopInFlight) return;
  stopInFlight = true;

  // The command returns once the worker has the stop flag, not once it has
  // exited, so the session is still running at this point. Keep the panel
  // locked and the phase on `listening`; the accepted Stop moves it to
  // `stopping`, and only the backend's `stopped` status unlocks it.
  setStatus("Stopping\u2026");

  try {
    const msg = await invoke("stop_live_transcription");
    log("stop_live_transcription:", msg);
    // `StopAccepted`: the worker is draining. Only while the phase is
    // still `listening` — a `stopped` that landed during the flight
    // already ended the session, and reviving `stopping` over it would
    // strand the panel on a worker that is gone.
    if (sessionPhase === "listening") {
      renderSessionPhase("stopping");
    }
  } catch (error) {
    // The backend refused the Stop, so the phase this UI holds is the
    // stale one. "No live transcription session is running" means the
    // worker already exited; only `get_session_phase` can say where that
    // left the session, and guessing `listening` here is what stranded the
    // panel on a session the backend no longer had.
    log("stop error:", error);
    setLiveError(`Failed to stop: ${error}`);
    await resyncSessionPhase();
  } finally {
    stopInFlight = false;
  }
}

// Listen for Tauri events emitted by the backend.
function setupEventListener() {
  if (!window.__TAURI__?.event?.listen) {
    console.warn("Tauri event API unavailable. Live transcription events will not be received.");
    return;
  }

  const subscription = window.__TAURI__.event.listen("app-event", (event) => {
    const payload = event.payload;
    log("app-event:", payload);

    switch (payload.kind) {
      case "status": {
        const status = payload.status;
        if (status) {
          setStatus(status.charAt(0).toUpperCase() + status.slice(1));
        }
        // The phase travels with the status (`UiAppEvent::phase`, from
        // `SessionPhase::apply_status`), so this draws the backend's answer
        // instead of replaying the table here. A payload without the field
        // predates #67; a terminal status still ends the session there,
        // which is the one outcome the panel cannot survive missing.
        if (typeof payload.phase === "string") {
          renderSessionPhase(payload.phase);
        } else if (status === "stopped" || status === "error") {
          renderSessionPhase("idle");
        }
        break;
      }
      case "transcript": {
        if (payload.text) {
          appendTranscript(payload.text, payload.duration_ms, payload.provider);
          setLiveError("");
        }
        break;
      }
      case "output": {
        if (payload.text) {
          appendTranslation(
            payload.text,
            payload.source_language,
            payload.target_language,
            payload.blocked,
          );
        }
        break;
      }
      case "speech": {
        showSpokenAudio(payload.text, payload.target_language, payload.duration_ms);
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

  // `listen` is a promise, and the Tauri ACL can reject it (for example
  // when no capability grants `core:event:allow-listen`). Ignoring that
  // rejection leaves the UI waiting forever for events that will never
  // arrive: status sticks on "Stopping…" and Stop appears broken. Surface
  // it instead, so a missing permission is visible rather than silent.
  if (subscription && typeof subscription.catch === "function") {
    subscription.catch((error) => {
      setStatus("Error");
      setLiveError(`Could not subscribe to backend events: ${error}`);
    });
  }
}

// ===== Existing mock / init flow ===========================================

async function syncInitialState() {
  if (!hasTauriInvoke()) {
    setStatus("Idle");
    if (transcriptEmpty) {
      transcriptEmpty.textContent = "Run the Tauri shell to activate the mock backend commands.";
    }
    if (translationEmpty) {
      translationEmpty.textContent = "The static preview is loaded, but invoke() is unavailable outside Tauri.";
    }
    return;
  }

  const status = await invoke("get_app_status");
  setStatus(status);
  setupEventListener();
  // A reloaded window must not start on the `idle` markup when the shell
  // still holds a session: the backend's slot outranks the page's default.
  await resyncSessionPhase();
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

  // Remembered until the device list arrives: the selector's options do
  // not exist yet, so the value cannot be applied here.
  let savedDevice = "";

  if (defaults && typeof defaults === "object") {
    if (typeof defaults.transcription_provider === "string") {
      providerSelect.value = defaults.transcription_provider;
    }
    if (typeof defaults.processor_provider === "string") {
      processorSelect.value = defaults.processor_provider;
    }
    // The mode round-trips: a stored respond config restores its host
    // names into the panel, so Stop + Start reproduces the same session
    // the settings file describes.
    if (defaults.mode && typeof defaults.mode === "object") {
      if (defaults.mode.kind === "respond") {
        modeSelect.value = "respond";
        const names = defaults.mode.policy && Array.isArray(defaults.mode.policy.host_names)
          ? defaults.mode.policy.host_names.join(", ")
          : "";
        hostNamesInput.value = names;
      } else {
        modeSelect.value = "translate";
      }
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
    if (Number.isFinite(defaults.hangover_ms)) {
      hangoverMsInput.value = String(defaults.hangover_ms);
    }
    if (Number.isFinite(defaults.min_speech_ms)) {
      minSpeechMsInput.value = String(defaults.min_speech_ms);
    }
    if (Number.isFinite(defaults.max_utterance_ms)) {
      maxUtteranceMsInput.value = String(defaults.max_utterance_ms);
    }
    if (typeof defaults.input_device === "string") {
      savedDevice = defaults.input_device;
    }
    if (defaults.audio_source === "system" || defaults.audio_source === "mic") {
      currentAudioSource = defaults.audio_source;
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

  // Input-device selector: the value is the device's stable id, the label is
  // the human name. Read once per load; a headset plugged in after the
  // window opened shows up next time the shell starts.
  //
  // A persisted id whose device is not connected right now is kept as an
  // option of its own. Without it the selector would sit on "System default"
  // and the next unrelated settings change would save `input_device: null`,
  // erasing the choice the moment the headset came back. Start still falls
  // back to the system default when the id cannot be opened.
  function keepSavedDevice() {
    if (!savedDevice) return;
    const known = Array.from(inputDeviceSelect.options).some(
      (option) => option.value === savedDevice,
    );
    if (!known) {
      const missing = document.createElement("option");
      missing.value = savedDevice;
      missing.textContent = `${savedDevice} (not connected)`;
      inputDeviceSelect.appendChild(missing);
      log("persisted input device is not available:", savedDevice);
    }
    inputDeviceSelect.value = savedDevice;
  }

  try {
    const devices = await invoke("list_input_devices");
    inputDeviceSelect.innerHTML = "";
    const systemDefault = document.createElement("option");
    systemDefault.value = "";
    systemDefault.textContent = "System default";
    inputDeviceSelect.appendChild(systemDefault);

    if (Array.isArray(devices)) {
      for (const device of devices) {
        const opt = document.createElement("option");
        opt.value = device.id;
        opt.textContent = device.name;
        inputDeviceSelect.appendChild(opt);
      }
    }
    log("loaded input devices:", devices);
  } catch (error) {
    log("could not load input devices:", error);
  }

  // Runs on both paths: a failed enumeration must not turn the saved
  // preference into null on the next save.
  keepSavedDevice();

  try {
    const processors = await invoke("get_supported_processor_providers");
    if (Array.isArray(processors) && processors.length > 0) {
      // Same rule as the provider list above: the backend owns the
      // valid names, the UI only renders them.
      processorSelect.innerHTML = "";
      for (const name of processors) {
        const opt = document.createElement("option");
        opt.value = name;
        opt.textContent = name;
        processorSelect.appendChild(opt);
      }
      processorSelect.value = defaults?.processor_provider ?? processorSelect.value;
      log("loaded supported processors:", processors);
    }
  } catch (error) {
    log("could not load supported processors:", error);
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
  // Persisting an out-of-range or empty box would store a config the
  // backend rejects on the next start; keep the last good values on disk
  // and let the field stay as the user typed it.
  const chunk = readNumberInput(chunkDurationMsInput, "Chunk duration", { integer: true });
  const threshold = readNumberInput(volumeThresholdInput, "Volume threshold");
  const endpointing = readEndpointInputs();
  if (!chunk.ok || !threshold.ok || !endpointing.ok) {
    const bad = [chunk, threshold, endpointing].find((result) => !result.ok);
    log("skipping settings save:", bad.message);
    return;
  }

  const config = {
    transcription_provider: providerSelect.value,
    processor_provider: processorSelect.value,
    source_language: sourceLanguage.value,
    target_language: targetLanguage.value,
    chunk_duration_ms: chunk.value,
    volume_threshold: threshold.value,
    hangover_ms: endpointing.hangover_ms,
    min_speech_ms: endpointing.min_speech_ms,
    max_utterance_ms: endpointing.max_utterance_ms,
    input_device: inputDeviceSelect.value || null,
    audio_source: currentAudioSource,
  };
  try {
    await invoke("save_runtime_settings", { config });
    log("saved runtime settings:", config);
  } catch (error) {
    // Don't block the UI on save failures; the user can still
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
  // Only the placeholder translation is rewritten: once a session has
  // appended rows, the rendered output belongs to those rows.
  if (transcriptEmpty && transcriptEmpty.hidden) {
    return;
  }

  if (translationEmpty) {
    translationEmpty.textContent = `[mock ${targetLanguage.value}] ${transcriptCopy}`;
  }
});

// ===== Local WAV transcription ============================================
//
// File-based only: no CPAL stream is started. Calls the
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

let isTranscribing = false;

async function runFileTranscribe() {
  // The click path is covered by the disabled attribute, but Enter inside
  // the path field and a rapid double-click can both land here.
  if (isTranscribing) return;

  const filePath = wavFilePathInput.value.trim();
  if (!filePath) {
    wavFilePathInput.setAttribute("aria-invalid", "true");
    setFileTranscribeStatus("No file \u2014 enter a WAV path first.", "error");
    setFileTranscribeError(
      "Enter the path to a WAV file, for example output/chunks/chunk_001.wav.",
    );
    wavFilePathInput.focus();
    return;
  }

  isTranscribing = true;
  wavFilePathInput.removeAttribute("aria-invalid");
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
    isTranscribing = false;
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
    if (transcribeFileButton.disabled) return;
    runFileTranscribe();
  }
});

window.addEventListener("DOMContentLoaded", () => {
  syncInitialState().catch((error) => {
    // Report the failure where alerts belong instead of overwriting the
    // transcript panels with a raw error string and losing the placeholder
    // copy that tells the user what to do next.
    setStatus("Error");
    setLiveError(`Could not read the backend state: ${error}`);
    renderSessionPhase("idle");
  });
});
