// Browser-level tests for the Tauri shell's frontend (`ui/`).
//
// What this covers: the session lifecycle the shell owns — Start sends a
// complete `AppRuntimeConfig`, the controls lock while a session runs, Stop
// releases them only once the backend reports `stopped`, and the `app-event`
// payloads (including `speech`, the utterance the backend just spoke) land in
// the right panels.
//
// What this does not cover: the Rust commands and the real microphone. Those
// are exercised by `cargo test --manifest-path src-tauri/Cargo.toml` and by the
// manual run in `docs/tauri-ui-shell.md`. The window itself is not automated
// because tauri-driver does not support macOS, so the suite talks to the same
// HTML/JS with a fake `window.__TAURI__` bridge installed before page scripts.
const { test, expect } = require("@playwright/test");

const DEFAULT_CONFIG = {
  transcription_provider: "mock-local",
  source_language: "en",
  target_language: "es",
  chunk_duration_ms: 1000,
  volume_threshold: 0.01,
  hangover_ms: 400,
  min_speech_ms: 200,
  max_utterance_ms: 15000,
};

// Runs in the page before `app.js`. `window.__tauriTest` is the inspection
// handle the tests use: every invoke is recorded, and `emitAppEvent` feeds the
// listener the frontend registered for `app-event`.
function installFakeBridge(page, config) {
  return page.addInitScript((initialConfig) => {
    const state = { calls: [], listeners: [] };
    window.__tauriTest = state;

    async function invoke(command, args) {
      state.calls.push({ command, args });
      switch (command) {
        case "get_app_status":
          return "Idle";
        case "load_runtime_settings":
        case "get_default_runtime_config":
          return initialConfig;
        case "get_supported_transcription_providers":
          return ["mock-local", "local-whisper"];
        case "start_live_transcription":
          return "Listening";
        case "stop_live_transcription":
          return "Stopped";
        default:
          return null;
      }
    }

    window.__TAURI__ = {
      core: { invoke },
      event: {
        listen: async (name, handler) => {
          state.listeners.push({ name, handler });
          return () => {};
        },
      },
    };

    window.emitAppEvent = (payload) => {
      for (const { name, handler } of state.listeners) {
        if (name === "app-event") handler({ payload });
      }
    };
  }, config);
}

async function openShell(page) {
  await installFakeBridge(page, DEFAULT_CONFIG);
  await page.goto("/");
  // `syncInitialState` is async: wait until it has pulled the persisted
  // settings, so the inputs hold the config the tests will assert on.
  await page.waitForFunction(() =>
    window.__tauriTest.calls.some((call) => call.command === "load_runtime_settings"),
  );
}

function findCall(page, command) {
  return page.evaluate(
    (name) => window.__tauriTest.calls.find((call) => call.command === name) ?? null,
    command,
  );
}

test("start sends the whole runtime config and locks the panel", async ({ page }) => {
  await openShell(page);

  await page.locator("#start-button").click();

  await expect(page.locator("#start-button")).toBeDisabled();
  await expect(page.locator("#stop-button")).toBeEnabled();

  const start = await findCall(page, "start_live_transcription");
  expect(start).not.toBeNull();
  expect(start.args.config).toEqual(DEFAULT_CONFIG);

  // The pill follows the backend's first event, not the command's return:
  // `start_live_transcription` resolves before the worker has started.
  await page.evaluate(() => window.emitAppEvent({ kind: "status", status: "listening" }));
  await expect(page.locator("#status-indicator")).toHaveText("Listening");

  // The panel is locked for the duration, endpointing fields included: a
  // mid-session edit would not reach the running worker.
  for (const selector of ["#chunk-duration-ms", "#hangover-ms", "#min-speech-ms", "#max-utterance-ms"]) {
    await expect(page.locator(selector)).toBeDisabled();
  }
});

test("utterance events fill the transcript, output and speech lines", async ({ page }) => {
  await openShell(page);
  await page.locator("#start-button").click();

  const spoken = "[mock es] mock translation: \"hola mundo\"";
  await page.evaluate((text) => {
    window.emitAppEvent({ kind: "status", status: "listening" });
    window.emitAppEvent({ kind: "transcript", index: 1, text: "hola mundo", provider: "mock-local" });
    window.emitAppEvent({
      kind: "output",
      index: 1,
      text,
      source_text: "hola mundo",
      source_language: "en",
      target_language: "es",
      is_final: true,
    });
    // One `speech` event per closed utterance; `duration_ms` comes from the
    // same pure helper the CLI prints.
    window.emitAppEvent({
      kind: "speech",
      index: 1,
      text,
      target_language: "es",
      duration_ms: 4320,
      is_final: true,
    });
  }, spoken);

  await expect(page.locator("#transcript-content")).toHaveText("hola mundo");
  await expect(page.locator("#translation-content")).toHaveText(spoken);
  await expect(page.locator("#speech-status")).toBeVisible();
  await expect(page.locator("#speech-status")).toHaveText(`Spoken · 4.3 s · es — ${spoken}`);
  await expect(page.locator("#live-error")).toBeHidden();
});

test("stop keeps the panel locked until the backend reports stopped", async ({ page }) => {
  await openShell(page);
  await page.locator("#start-button").click();
  await expect(page.locator("#start-button")).toBeDisabled();

  await page.locator("#stop-button").click();

  // The command returns once the worker has the stop flag, not once it has
  // exited: only the `stopped` status event may unlock the panel.
  await expect(page.locator("#status-indicator")).toHaveText("Stopping…");
  await expect(page.locator("#stop-button")).toBeDisabled();
  await expect(page.locator("#start-button")).toBeDisabled();
  await expect(page.locator("#hangover-ms")).toBeDisabled();
  expect(await findCall(page, "stop_live_transcription")).not.toBeNull();

  await page.evaluate(() => window.emitAppEvent({ kind: "status", status: "stopped" }));

  await expect(page.locator("#status-indicator")).toHaveText("Stopped");
  await expect(page.locator("#start-button")).toBeEnabled();
  await expect(page.locator("#stop-button")).toBeDisabled();
  await expect(page.locator("#hangover-ms")).toBeEnabled();
});

test("a second click while starting does not spawn a second session", async ({ page }) => {
  await openShell(page);

  const start = page.locator("#start-button");
  await start.click();
  // The button is disabled by the first click; a forced second click must not
  // reach the backend either.
  await start.dispatchEvent("click");
  await page.waitForTimeout(50);

  const starts = await page.evaluate(
    () => window.__tauriTest.calls.filter((call) => call.command === "start_live_transcription").length,
  );
  expect(starts).toBe(1);
});

test("an impossible endpointing pair blocks the start", async ({ page }) => {
  await openShell(page);

  await page.locator("#min-speech-ms").fill("3000");
  await page.locator("#max-utterance-ms").fill("1000");
  await page.locator("#start-button").click();

  await expect(page.locator("#live-error")).toBeVisible();
  await expect(page.locator("#live-error")).toContainText("Maximum utterance");
  await expect(page.locator("#max-utterance-ms")).toHaveAttribute("aria-invalid", "true");
  expect(await findCall(page, "start_live_transcription")).toBeNull();
  await expect(page.locator("#start-button")).toBeEnabled();
});
