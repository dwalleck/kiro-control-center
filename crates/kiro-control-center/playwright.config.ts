import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/e2e",
  timeout: 60_000,
  retries: 0,
  use: {
    baseURL: "http://localhost:1420",
    trace: "retain-on-failure",
  },
  webServer: {
    command: "cargo tauri dev",
    url: "http://localhost:1420",
    timeout: 120_000,
    reuseExistingServer: !process.env.CI,
  },
});
