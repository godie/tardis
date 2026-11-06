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
  processor_provider: "mock",
  mode: { kind: "translate" },
  source_language: "en",
  target_language: "es",
  chunk_duration_ms: 1000,
  volume_threshold: 0.01,
  hangover_ms: 400,
  min_speech_ms: 200,
  max_utterance_ms: 15000,
  input_device: null,
};

// Two fake input devices: the selector shows `name` and persists `id`.
const INPUT_DEVICES = [
  { id: "coreaudio:builtin", name: "Built-in Microphone" },
  { id: "coreaudio:usb", name: "USB Audio" },
];

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
        case "list_input_devices":
          return [
            { id: "coreaudio:builtin", name: "Built-in Microphone" },
            { id: "coreaudio:usb", name: "USB Audio" },
          ];
        case "get_supported_processor_providers":
          return ["mock", "llm"];
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

test("the input-device selector lists devices and sends the chosen id", async ({ page }) => {
  await openShell(page);

  const select = page.locator("#input-device");
  // "System default" plus one row per device the backend reported.
  await expect(select.locator("option")).toHaveCount(1 + INPUT_DEVICES.length);
  await expect(select).toHaveValue("");
  await expect(select.locator("option").nth(1)).toHaveText("Built-in Microphone");

  await select.selectOption("coreaudio:usb");
  await page.locator("#start-button").click();

  const start = await findCall(page, "start_live_transcription");
  expect(start.args.config.input_device).toBe("coreaudio:usb");
});

test("a persisted device is preselected, and a missing one stays selected", async ({
  page,
}) => {
  // The saved id exists: the selector must show it, not "System default".
  await installFakeBridge(page, { ...DEFAULT_CONFIG, input_device: "coreaudio:builtin" });
  await page.goto("/");
  await page.waitForFunction(() =>
    window.__tauriTest.calls.some((call) => call.command === "list_input_devices"),
  );
  await expect(page.locator("#input-device")).toHaveValue("coreaudio:builtin");

  // The saved id is a device that is gone (unplugged headset). The selector
  // keeps it as a "not connected" option: the backend already falls back to
  // the system default at Start, and an unrelated settings change must not
  // erase the choice before the headset comes back.
  await installFakeBridge(page, { ...DEFAULT_CONFIG, input_device: "coreaudio:gone" });
  await page.goto("/");
  await page.waitForFunction(() =>
    window.__tauriTest.calls.some((call) => call.command === "list_input_devices"),
  );
  const select = page.locator("#input-device");
  await expect(select).toHaveValue("coreaudio:gone");
  await expect(select.locator("option[value='coreaudio:gone']")).toHaveText(
    "coreaudio:gone (not connected)",
  );

  // Changing something unrelated must persist the same id, not null.
  await page.locator("#target-language").selectOption("it");
  await page.waitForFunction(() =>
    window.__tauriTest.calls.some((call) => call.command === "save_runtime_settings"),
  );
  const save = await findCall(page, "save_runtime_settings");
  expect(save.args.config.input_device).toBe("coreaudio:gone");
});

test("a failed device enumeration does not erase the saved id", async ({ page }) => {
  await installFakeBridge(page, { ...DEFAULT_CONFIG, input_device: "coreaudio:usb" });
  await page.addInitScript(() => {
    window.__TAURI__.core.invoke = ((invoke) =>
      async (command, args) => {
        if (command === "list_input_devices") {
          throw new Error("could not enumerate input devices");
        }
        return invoke(command, args);
      })(window.__TAURI__.core.invoke);
  });
  await page.goto("/");
  await page.waitForFunction(() =>
    window.__tauriTest.calls.some((call) => call.command === "load_runtime_settings"),
  );

  await page.locator("#target-language").selectOption("it");
  await page.waitForFunction(() =>
    window.__tauriTest.calls.some((call) => call.command === "save_runtime_settings"),
  );
  const save = await findCall(page, "save_runtime_settings");
  expect(save.args.config.input_device).toBe("coreaudio:usb");
});

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
  for (const selector of ["#chunk-duration-ms", "#hangover-ms", "#min-speech-ms", "#max-utterance-ms", "#mode-select", "#host-names"]) {
    await expect(page.locator(selector)).toBeDisabled();
  }
});

test("utterance events fill the transcript, output and speech lines", async ({ page }) => {
  await openShell(page);
  await page.locator("#start-button").click();

  const spoken = "[mock es] mock translation: \"hola mundo\"";
  await page.evaluate((text) => {
    window.emitAppEvent({ kind: "status", status: "listening" });
    window.emitAppEvent({
      kind: "transcript",
      index: 1,
      text: "hola mundo",
      provider: "mock-local",
      duration_ms: 4320,
      is_final: true,
    });
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

  const row = page.locator("#transcript-content .history-item").first();
  await expect(row.locator(".history-text")).toHaveText("hola mundo");
  // Duration and provider come from the event, so the row can be read later
  // without re-listening to the audio.
  await expect(row.locator(".history-meta")).toHaveText("4.3 s \u00b7 mock-local");
  await expect(row.locator(".history-time")).not.toBeEmpty();
  // The rendered output gets the same treatment: a row of its own, tagged
  // with the language pair that produced it.
  const rendered = page.locator("#translation-content .history-item").first();
  await expect(rendered.locator(".history-text")).toHaveText(spoken);
  await expect(rendered.locator(".history-meta")).toHaveText("en \u2192 es");
  await expect(page.locator("#translation-empty")).toBeHidden();
  await expect(page.locator("#speech-status")).toBeVisible();
  await expect(page.locator("#speech-status")).toHaveText(`Spoken · 4.3 s · es — ${spoken}`);
  await expect(page.locator("#live-error")).toBeHidden();
});

test("each utterance appends to the transcript and translation histories instead of replacing them", async ({ page }) => {
  await openShell(page);
  await page.locator("#start-button").click();

  await page.evaluate(() => {
    window.emitAppEvent({ kind: "status", status: "listening" });
    window.emitAppEvent({
      kind: "transcript",
      index: 1,
      text: "first thing heard",
      provider: "mock-local",
      duration_ms: 1000,
      is_final: true,
    });
    window.emitAppEvent({
      kind: "transcript",
      index: 2,
      text: "second thing heard",
      provider: "local-whisper",
      duration_ms: 2500,
      is_final: true,
    });
    window.emitAppEvent({
      kind: "output",
      index: 1,
      text: "lo primero oido",
      source_language: "en",
      target_language: "es",
      output_kind: "translate",
      is_final: true,
    });
    window.emitAppEvent({
      kind: "output",
      index: 2,
      text: "lo segundo oido",
      source_language: "en",
      target_language: "es",
      output_kind: "translate",
      is_final: true,
    });
  });

  const rows = page.locator("#transcript-content .history-item");
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0).locator(".history-text")).toHaveText("first thing heard");
  await expect(rows.nth(1).locator(".history-text")).toHaveText("second thing heard");
  await expect(rows.nth(1).locator(".history-meta")).toHaveText("2.5 s \u00b7 local-whisper");

  // Same for the rendered lines: the second must not replace the first.
  const rendered = page.locator("#translation-content .history-item");
  await expect(rendered).toHaveCount(2);
  await expect(rendered.nth(0).locator(".history-text")).toHaveText("lo primero oido");
  await expect(rendered.nth(1).locator(".history-text")).toHaveText("lo segundo oido");

  // The empty-state hints are gone once there is something to read.
  await expect(page.locator("#transcript-empty")).toBeHidden();
  await expect(page.locator("#translation-empty")).toBeHidden();
});

test("history panels keep only the newest rows", async ({ page }) => {
  await openShell(page);
  await page.locator("#start-button").click();

  // A long session must not grow the webview's DOM without bound. The file
  // on disk still gets every record; the panels keep the recent tail.
  await page.evaluate(() => {
    window.emitAppEvent({ kind: "status", status: "listening" });
    for (let i = 1; i <= 205; i += 1) {
      window.emitAppEvent({
        kind: "transcript",
        index: i,
        text: `utterance ${i}`,
        provider: "mock-local",
        duration_ms: 1000,
        is_final: true,
      });
    }
  });

  const rows = page.locator("#transcript-content .history-item");
  await expect(rows).toHaveCount(200);
  // The oldest rows are the ones retired; the newest is still last.
  await expect(rows.first().locator(".history-text")).toHaveText("utterance 6");
  await expect(rows.last().locator(".history-text")).toHaveText("utterance 205");
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

// The Tauri 2 ACL rejects `event.listen` unless a capability grants
// `core:event:allow-listen`. Swallowing that rejection is what makes the shell
// wait forever for a `stopped` event that can never arrive (the "stuck on
// Stopping…" symptom). The failure has to be visible, not silent.
test("a rejected event subscription surfaces an error instead of hanging", async ({ page }) => {
  await installFakeBridge(page, DEFAULT_CONFIG);
  await page.addInitScript(() => {
    window.__TAURI__.event.listen = async () => {
      throw new Error(
        "event.listen not allowed. Permissions associated with this command: core:event:allow-listen",
      );
    };
  });
  await page.goto("/");
  await page.waitForFunction(() =>
    window.__tauriTest.calls.some((call) => call.command === "load_runtime_settings"),
  );

  await expect(page.locator("#live-error")).toBeVisible();
  await expect(page.locator("#live-error")).toContainText("Could not subscribe to backend events");
  await expect(page.locator("#status-indicator")).toHaveText("Error");
});
