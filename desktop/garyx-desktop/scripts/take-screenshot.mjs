#!/usr/bin/env node
/**
 * take-screenshot.mjs — Capture a screenshot of the Garyx app.
 *
 * Usage:
 *   node scripts/take-screenshot.mjs [output-path] [--cdp PORT] [--wait SELECTOR] [--navigate PATH]
 *
 * Modes:
 *   1. CDP connect (preferred): Connect to a running Garyx via --remote-debugging-port
 *      Set GARY_CDP_PORT env or pass --cdp PORT (default: 9229)
 *   2. Playwright launch (fallback): Start a fresh Electron instance against the real gateway
 *
 * Examples:
 *   # Connect to running app on CDP port 9229
 *   node scripts/take-screenshot.mjs /tmp/screenshot.png --cdp 9229
 *
 *   # Launch fresh instance, navigate to team chat, wait for element
 *   node scripts/take-screenshot.mjs /tmp/team-chat.png --navigate /team-chat --wait .team-chat-view
 *
 *   # Simple full-page screenshot with Playwright launch
 *   node scripts/take-screenshot.mjs /tmp/app.png
 */

import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { mkdir } from 'node:fs/promises';
import { _electron as electron, chromium } from 'playwright';
import { buildDesktopElectronLaunchEnv } from './electron-launch-env.mjs';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const projectDir = path.resolve(scriptDir, '..');

function parseArgs(argv) {
  const args = {
    output: '/tmp/garyx-screenshot.png',
    cdpPort: parseInt(process.env.GARY_CDP_PORT || '0', 10),
    waitSelector: null,
    navigate: null,
    timeout: 10000,
  };

  let i = 2; // skip node and script path
  while (i < argv.length) {
    const arg = argv[i];
    if (arg === '--cdp') {
      args.cdpPort = parseInt(argv[++i], 10);
    } else if (arg === '--wait') {
      args.waitSelector = argv[++i];
    } else if (arg === '--navigate') {
      args.navigate = argv[++i];
    } else if (arg === '--timeout') {
      args.timeout = parseInt(argv[++i], 10);
    } else if (!arg.startsWith('--')) {
      args.output = arg;
    }
    i++;
  }
  return args;
}

async function screenshotViaCDP(args) {
  const url = `http://127.0.0.1:${args.cdpPort}`;
  console.log(`Connecting to CDP at ${url}...`);

  const browser = await chromium.connectOverCDP(url);
  const contexts = browser.contexts();
  if (contexts.length === 0) {
    throw new Error('No browser contexts found via CDP');
  }

  const pages = contexts[0].pages();
  if (pages.length === 0) {
    throw new Error('No pages found via CDP');
  }

  const page = pages[0];
  console.log(`Connected to page: ${page.url()}`);

  if (args.navigate) {
    const currentUrl = new URL(page.url());
    const targetUrl = `${currentUrl.origin}${args.navigate}`;
    console.log(`Navigating to ${targetUrl}...`);
    await page.goto(targetUrl, { waitUntil: 'domcontentloaded' });
  }

  if (args.waitSelector) {
    console.log(`Waiting for selector: ${args.waitSelector}...`);
    await page.waitForSelector(args.waitSelector, { timeout: args.timeout });
  }

  await page.screenshot({ path: args.output, fullPage: false });
  console.log(`Screenshot saved to ${args.output}`);

  await browser.close();
  return args.output;
}

async function screenshotViaLaunch(args) {
  console.log('Launching Garyx via Playwright...');

  const electronApp = await electron.launch({
    args: [projectDir],
    cwd: projectDir,
    env: buildDesktopElectronLaunchEnv(),
  });

  try {
    const window = await electronApp.firstWindow();
    await window.waitForLoadState('domcontentloaded');
    await window.setViewportSize({ width: 1480, height: 940 });

    // Wait for the app shell to render
    try {
      await window.locator('.app-shell').waitFor({ timeout: args.timeout });
    } catch {
      console.warn('Warning: .app-shell not found within timeout, taking screenshot anyway');
    }

    if (args.navigate) {
      console.log(`Navigating to ${args.navigate}...`);
      // Use client-side navigation
      await window.evaluate((path) => {
        window.location.hash = path;
      }, args.navigate);
      // Give the route time to render
      await new Promise((resolve) => setTimeout(resolve, 2000));
    }

    if (args.waitSelector) {
      console.log(`Waiting for selector: ${args.waitSelector}...`);
      await window.waitForSelector(args.waitSelector, { timeout: args.timeout });
    }

    // Try .app-shell first, fall back to full page
    try {
      await window.locator('.app-shell').screenshot({ path: args.output });
    } catch {
      await window.screenshot({ path: args.output });
    }

    console.log(`Screenshot saved to ${args.output}`);
    return args.output;
  } finally {
    await electronApp.close();
  }
}

async function main() {
  const args = parseArgs(process.argv);

  // Ensure output directory exists
  await mkdir(path.dirname(args.output), { recursive: true });

  console.log(`Output: ${args.output}`);

  if (args.cdpPort > 0) {
    try {
      return await screenshotViaCDP(args);
    } catch (error) {
      console.warn(`CDP connection failed (port ${args.cdpPort}): ${error.message}`);
      console.log('Falling back to Playwright launch...');
    }
  }

  return await screenshotViaLaunch(args);
}

main().catch((error) => {
  console.error(`Screenshot failed: ${error.message}`);
  process.exit(1);
});
