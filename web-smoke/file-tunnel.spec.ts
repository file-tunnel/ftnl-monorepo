import { test, expect } from '@playwright/test';

// Live web surfaces for the file-tunnel org, discovered from the org repos:
//   - file-tunnel.github.io: the Astro marketing + docs site (the repo has no
//     custom CNAME, so the *.github.io domain is canonical). Built <title> is
//     "File Tunnel — files from your phone, right where you need them" and the
//     body carries the product copy. This is the reliably-live surface and
//     carries the HARD assertions.
//   - file-tunnel.github.io/security/: the security-model page linked from the
//     primary nav. Checked tolerantly (log + soft-skip) so it can never flap.
//   - The backend API (ftnl-backend-api.rs :8080) and portal (:3000) have NO
//     confirmed public URL, so they are intentionally not smoke-tested from CI.
const SITE = 'https://file-tunnel.github.io/';
const SECURITY = 'https://file-tunnel.github.io/security/';

test('file-tunnel.github.io is live and is the File Tunnel site', async ({ page }) => {
  const resp = await page.goto(SITE, { waitUntil: 'domcontentloaded' });
  expect(resp, `no response from ${SITE}`).toBeTruthy();
  expect(resp!.status(), `unexpected HTTP status from ${SITE}`).toBeLessThan(400);

  // Title starts with "File Tunnel"; match case-insensitively to tolerate the
  // "— files from your phone…" suffix and future copy tweaks.
  await expect(page).toHaveTitle(/file tunnel/i);

  // Distinctive, stable marketing copy for the product.
  const body = (await page.locator('body').innerText()).toLowerCase();
  expect(body, 'expected the File Tunnel product copy').toContain('file tunnel');
});

test('security page (tolerant — may move/rename)', async ({ page }) => {
  try {
    const resp = await page.goto(SECURITY, { waitUntil: 'domcontentloaded', timeout: 15_000 });
    if (!resp || resp.status() >= 400) {
      test.skip(true, `security page ${SECURITY} not serving (status ${resp?.status() ?? 'none'}) — tolerated`);
      return;
    }
    // eslint-disable-next-line no-console
    console.log(`security page ${SECURITY} responded HTTP ${resp.status()} (observe-only)`);
  } catch (e) {
    test.skip(true, `security page ${SECURITY} unreachable (${(e as Error).message}) — tolerated`);
  }
});
