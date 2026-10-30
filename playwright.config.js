// @ts-check
const { defineConfig, devices } = require("@playwright/test");

// The suite drives the shell's frontend (`ui/`) over plain HTTP with a fake
// `window.__TAURI__` bridge injected before the page scripts run. The real
// window is not automated: tauri-driver does not support macOS, and the Rust
// side has its own tests. See `ui/tests/session.spec.js`.
module.exports = defineConfig({
  testDir: "./ui/tests",
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? "list" : "list",
  use: {
    baseURL: "http://127.0.0.1:4173",
    trace: "on-first-retry",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: {
    command: "node ui/tests/serve.mjs",
    url: "http://127.0.0.1:4173/index.html",
    reuseExistingServer: !process.env.CI,
    timeout: 30_000,
  },
});
