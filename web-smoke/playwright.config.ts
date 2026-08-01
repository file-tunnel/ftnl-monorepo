import { defineConfig, devices } from '@playwright/test';

// Minimal config for the org's live browser smoke. No local web server: every
// test navigates to a public URL. Retries absorb transient network blips so a
// scheduled run does not flap.
export default defineConfig({
  testDir: '.',
  timeout: 30_000,
  expect: { timeout: 10_000 },
  retries: 2,
  reporter: [['list']],
  use: {
    ...devices['Desktop Chrome'],
    headless: true,
  },
});
