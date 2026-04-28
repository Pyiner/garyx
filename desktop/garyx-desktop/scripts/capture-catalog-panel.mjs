#!/usr/bin/env node
/**
 * Take a screenshot of the ChannelPluginCatalogPanel.
 *
 * Workflow the team uses:
 *   1. Start a mock gateway answering `GET /api/channels/plugins`.
 *   2. Launch Electron via Playwright's `_electron.launch()` — it
 *      handles the CJS/ESM bootstrap headaches a raw `spawn(electron)`
 *      hits. The app's own `browser-runtime.ts` opens a CDP listener
 *      on the project-default port 39222 (we do NOT set
 *      `GARYX_DESKTOP_DISABLE_FIXED_CDP`).
 *   3. Wait for the renderer to load.
 *   4. Invoke the `playwright-cli` binary to attach over CDP, drive
 *      Settings → Channels, and screenshot the panel. Keeping the
 *      driver in a separate subprocess makes it easy to swap in
 *      manual `playwright-cli <cmd>` calls when debugging.
 *
 * Output: `test-artifacts/catalog-panel.png`.
 */
import { spawnSync } from 'node:child_process';
import { createServer } from 'node:http';
import { mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { _electron as electron } from 'playwright';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const projectDir = path.resolve(scriptDir, '..');
const OUT_DIR = path.resolve(projectDir, 'test-artifacts');
const OUT_FILE = path.join(OUT_DIR, 'catalog-panel.png');
// Playwright's `_electron.launch()` forces `--remote-debugging-port=0`
// on the command line, which means the project's own
// `browser-runtime.ts` fallback (fixed port 39222) is NOT applied —
// the OS assigns a random port. We query that port via
// `app.evaluate(() => ...)` after launch so playwright-cli attaches
// to OUR isolated instance, not whatever already owns 39222 on the
// developer machine.
// Short: macOS caps UNIX socket paths at ~104 chars and playwright-cli
// appends a session-name-based suffix in a deep /var/folders/... path.
const SESSION_NAME = 'gx';

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function resolveCdpPort(userDataDir, timeoutMs = 10_000) {
  const portFile = path.join(userDataDir, 'DevToolsActivePort');
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const raw = await readFile(portFile, 'utf8');
      // Format: `<port>\n<websocketPath>\n`. First line is what we need.
      const first = raw.split(/\r?\n/)[0].trim();
      const port = Number(first);
      if (Number.isFinite(port) && port > 0) return port;
    } catch {
      // File hasn't landed yet; poll.
    }
    await sleep(100);
  }
  throw new Error(`DevToolsActivePort never appeared under ${userDataDir}`);
}

const CATALOG_RESPONSE = {
  ok: true,
  plugins: [
    {
      id: 'telegram',
      display_name: 'Telegram',
      version: '0.1.4',
      description: 'Built-in Telegram channel runtime',
      state: 'ready',
      capabilities: {
        outbound: true, inbound: true, streaming: false, images: false, files: false,
        delivery_model: 'pull_explicit_ack',
      },
      schema: { type: 'object', required: ['token'] },
      auth_flows: [],
      accounts: [
        { id: 'novelist_bot', enabled: true, config: { token: '***' } },
        { id: 'debug_bot', enabled: false, config: { token: '***' } },
      ],
    },
    {
      id: 'feishu',
      display_name: 'Feishu / Lark',
      version: '0.1.4',
      description: 'Built-in Feishu/Lark channel runtime',
      state: 'ready',
      capabilities: {
        outbound: true, inbound: true, streaming: false, images: false, files: false,
        delivery_model: 'pull_explicit_ack',
      },
      schema: { type: 'object', required: ['app_id', 'app_secret'] },
      auth_flows: [{ id: 'device_code', label: 'OAuth device code', prompt: 'Scan QR' }],
      accounts: [{ id: 'tenant_alpha', enabled: true, config: {} }],
    },
    {
      id: 'weixin',
      display_name: 'Weixin (WeChat)',
      version: '0.1.4',
      description: 'Built-in Weixin channel runtime',
      state: 'ready',
      capabilities: {
        outbound: true, inbound: true, streaming: false, images: false, files: false,
        delivery_model: 'pull_explicit_ack',
      },
      schema: { type: 'object', required: ['token', 'uin'] },
      auth_flows: [{ id: 'qr_code', label: 'WeChat QR login', prompt: 'Scan QR' }],
      accounts: [],
    },
    {
      id: 'acmechat',
      display_name: 'AcmeChat',
      version: '0.1.0',
      description: 'AcmeChat subprocess channel plugin (polls agent-messages API).',
      state: 'running',
      capabilities: {
        outbound: true, inbound: true, streaming: false, images: false, files: false,
        delivery_model: 'pull_explicit_ack',
      },
      schema: { type: 'object', required: ['token'] },
      auth_flows: [],
      accounts: [{ id: 'product_ship', enabled: true, config: {} }],
      icon_data_url:
        'data:image/svg+xml;base64,' +
        Buffer.from(
          '<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32">' +
            '<rect width="32" height="32" rx="6" fill="#00a240"/>' +
            '<text x="16" y="22" text-anchor="middle" font-family="-apple-system" font-size="18" fill="white" font-weight="700">M</text>' +
            '</svg>',
        ).toString('base64'),
    },
  ],
};

const UPSTREAM = 'http://127.0.0.1:31337';

async function createMockGateway() {
  // Thin reverse proxy that intercepts ONLY `/api/channels/plugins`
  // (with our pre-baked catalog) and forwards everything else to
  // the developer's real gateway on 31337. This gives the desktop
  // app a fully-working backend — threads, bots, agents, the works
  // — while letting us exercise the Channels panel's new schema-
  // driven data path without having to mock every endpoint the app
  // pokes during boot.
  const server = createServer(async (req, res) => {
    const { pathname } = new URL(req.url, `http://${req.headers.host}`);
    if (pathname === '/api/channels/plugins' && req.method === 'GET') {
      res.statusCode = 200;
      res.setHeader('content-type', 'application/json');
      res.end(JSON.stringify(CATALOG_RESPONSE));
      return;
    }
    // Forward the rest.
    const upstreamUrl = `${UPSTREAM}${req.url}`;
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    const body = chunks.length ? Buffer.concat(chunks) : undefined;
    try {
      const upstreamRes = await fetch(upstreamUrl, {
        method: req.method,
        headers: Object.fromEntries(
          Object.entries(req.headers).filter(
            ([k]) => k !== 'host' && k !== 'content-length',
          ),
        ),
        body,
        redirect: 'manual',
      });
      res.statusCode = upstreamRes.status;
      upstreamRes.headers.forEach((value, key) => {
        if (key === 'content-encoding' || key === 'transfer-encoding') return;
        res.setHeader(key, value);
      });
      const upstreamBody = Buffer.from(await upstreamRes.arrayBuffer());
      res.end(upstreamBody);
    } catch (err) {
      res.statusCode = 502;
      res.end(`upstream unreachable: ${err.message}`);
    }
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const { port } = server.address();
  return {
    gatewayUrl: `http://127.0.0.1:${port}`,
    close: () => new Promise((r) => server.close(() => r())),
  };
}

async function prepareIsolatedHome(gatewayUrl) {
  const home = await mkdtemp(path.join(os.tmpdir(), 'garyx-catalog-'));
  const userDataDir = path.join(home, 'electron-user-data');
  await mkdir(userDataDir, { recursive: true });
  // The Electron app reads its settings from
  // `app.getPath('userData')/garyx-desktop-state.json` (see
  // `src/main/store.ts`). Writing here — NOT `$HOME/.garyx/*` —
  // is what makes our mock gateway URL take effect.
  const settingsFile = path.join(userDataDir, 'garyx-desktop-state.json');
  // Top-level shape is `{ settings: {...}, workspaces: [...] }`
  // (see `src/main/store.ts`). A flat object gets silently wrapped
  // and our custom gatewayUrl dropped to the app's default.
  await writeFile(
    settingsFile,
    JSON.stringify(
      {
        settings: {
          gatewayUrl,
          accountId: 'catalog-smoke',
          fromId: 'catalog-smoke',
          agentId: 'claude',
          timeoutSeconds: 60,
          providerClaudeEnv: '',
          providerCodexEnv: '',
        },
        workspaces: [{ id: 'workspace::catalog', name: 'Catalog', path: home }],
        selectedWorkspaceId: 'workspace::catalog',
      },
      null,
      2,
    ),
  );
  console.log(`[capture] wrote settings → ${settingsFile}`);
  return { home, userDataDir, settingsFile };
}

function pcli(...args) {
  const result = spawnSync('playwright-cli', ['-s', SESSION_NAME, ...args], {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(
      `playwright-cli ${args.join(' ')} exited ${result.status}\nstdout: ${result.stdout}\nstderr: ${result.stderr}`,
    );
  }
  return result.stdout;
}

async function main() {
  // playwright-cli spawns a long-lived per-session daemon on first
  // `attach`. If a previous run died before closing, the daemon
  // stays attached to the OLD CDP endpoint (likely the developer's
  // running Garyx) and every subsequent command goes to the wrong
  // browser. Close any lingering session with our name up front.
  spawnSync('playwright-cli', ['-s', SESSION_NAME, 'close'], { stdio: 'ignore' });

  const gateway = await createMockGateway();
  console.log(`[capture] mock gateway on ${gateway.gatewayUrl}`);
  const { home, userDataDir } = await prepareIsolatedHome(gateway.gatewayUrl);

  // Launch Electron. Playwright's `_electron.launch()` sidesteps the
  // CJS/ESM interop bug that a raw `spawn(electron, ...)` hits on
  // `out/main/index.js`. The app's own CDP listener (port 39222)
  // opens because we don't disable it.
  // Pass `--user-data-dir` on the command line so Chromium's native
  // layer uses it BEFORE the app's own `GARYX_DESKTOP_USER_DATA_PATH`
  // env-based override runs. Without this, `DevToolsActivePort`
  // lands in the default user-data location (shared with any
  // other running Garyx instance on the dev machine), and
  // `playwright-cli attach --cdp` hits the wrong app.
  const electronApp = await electron.launch({
    args: [projectDir, `--user-data-dir=${userDataDir}`],
    cwd: projectDir,
    env: { ...process.env, HOME: home, GARYX_DESKTOP_USER_DATA_PATH: userDataDir },
  });
  let firstWindow = null;
  try {
    // `electronApp.firstWindow()` races; we want the MAIN renderer
    // (`index.html`), not the built-in BrowserPage tab that Garyx
    // opens to google.com by default. Iterate the windows, pick
    // the one whose URL points at our renderer bundle.
    const pickMain = async () => {
      for (const w of electronApp.windows()) {
        const url = w.url();
        if (url.includes('index.html') || url.startsWith('file://')) return w;
      }
      return null;
    };
    for (let attempt = 0; attempt < 40; attempt += 1) {
      firstWindow = await pickMain();
      if (firstWindow) break;
      await sleep(250);
    }
    if (!firstWindow) throw new Error('Garyx renderer window never appeared');
    await firstWindow.waitForLoadState('domcontentloaded');
    await firstWindow.bringToFront();
    console.log('[capture] renderer ready; resolving CDP port for playwright-cli');

    // Chromium writes the OS-assigned CDP port into
    // `<userDataDir>/DevToolsActivePort` once the server is up.
    // Poll until it materialises — usually instant once the
    // renderer has domcontentloaded.
    const cdpPort = await resolveCdpPort(userDataDir);
    const cdpEndpoint = `http://127.0.0.1:${cdpPort}`;
    console.log(`[capture] CDP endpoint: ${cdpEndpoint}`);

    // Attach via playwright-cli CLI over CDP to our instance.
    pcli('attach', '--cdp', cdpEndpoint);
    console.log('[capture] playwright-cli attached via CDP');

    // Drive the UI: Settings → Channels.
    // playwright-cli has no "getByRole"; use plain selectors.
    await mkdir(OUT_DIR, { recursive: true });

    // Use playwright (node-side) to drive clicks because the CLI's
    // `click` takes an element ref from a snapshot which is heavier.
    // playwright-cli's job here is the screenshot only — this is
    // still "connect via CDP and screenshot" but clicks happen
    // through the same CDP session Playwright's ElectronApp holds.
    // Settings lives in the left sidebar as a link-like div/button,
    // not necessarily a <button role=button> — match on visible
    // text, filtering to an element whose content is exactly
    // "Settings" (not something like "Open Settings").
    const settingsEntry = firstWindow
      .locator('text=/^\\s*(Settings|设置)\\s*$/')
      .first();
    await settingsEntry.waitFor({ timeout: 10_000 });
    await settingsEntry.click();
    console.log('[capture] Settings opened');
    await sleep(600);

    const channelsTab = firstWindow
      .locator('text=/^\\s*(Channels|渠道)\\s*$/')
      .first();
    await channelsTab.waitFor({ timeout: 10_000 });
    await channelsTab.click();
    console.log('[capture] Channels tab opened');
    await sleep(2500);

    // Diagnostic: dump what the page actually rendered in the
    // channels tab so we can tell whether the catalog hook ran at
    // all. Useful when the UI is gated by an upstream load state.
    const panelCount = await firstWindow.locator('.channel-plugin-catalog').count();
    console.log(`[capture] .channel-plugin-catalog matches: ${panelCount}`);
    if (panelCount === 0) {
      const titles = await firstWindow
        .locator('.channel-plugin-catalog-title, .codex-section-title')
        .allInnerTexts();
      console.log(`[capture] visible section titles: ${JSON.stringify(titles)}`);
    } else {
      await firstWindow.locator('.channel-plugin-catalog').scrollIntoViewIfNeeded();
    }

    await mkdir(OUT_DIR, { recursive: true });
    // Make sure playwright-cli is pointed at the Garyx renderer
    // (it may default to the first CDP target, which is the
    // built-in BrowserPage tab pointing at google.com).
    const tabs = pcli('tab-list');
    console.log(`[capture] tab-list:\n${tabs}`);
    const garyxTabMatch = tabs.match(/^\s*-\s+(\d+):.*Garyx.*file:\/\//m);
    if (garyxTabMatch) {
      pcli('tab-select', garyxTabMatch[1]);
      console.log(`[capture] selected Garyx tab ${garyxTabMatch[1]}`);
    }
    pcli('screenshot', '--filename', OUT_FILE, '--full-page');
    console.log(`[capture] wrote ${OUT_FILE}`);
  } finally {
    try {
      pcli('close');
    } catch {
      /* closing an unattached session errors; ignore */
    }
    if (electronApp) await electronApp.close().catch(() => {});
    await gateway.close();
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
