#!/usr/bin/env node

import assert from 'node:assert/strict';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { chromium } from 'playwright';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const projectDir = path.resolve(scriptDir, '..');
const fixturePath = path.join(
  projectDir,
  'src/renderer/src/app-shell/fixtures/legacy-horizontal-layout-oracle.json',
);
const defaultCdpEndpoint = 'http://127.0.0.1:39222';
const expectedViewport = { width: 1480, height: 940 };
const settleDelayMs = 240;
const defaultThreadLogsPanelWidth = 360;
const overlayThreadLogsPanelWidth = 480;
const sideToolsToggleSelector =
  '.conversation-header-actions > button.conversation-header-action-icon';

const elementSelectors = {
  appShell: '.app-shell',
  sidebarToggle: '.app-shell > button.sidebar-collapse-toggle',
  leftRail: '.left-rail',
  conversationRail:
    '.bot-conversation-rail, .workspace-conversation-rail, .recent-conversation-rail',
  conversation: '.conversation',
  conversationHeader: '.conversation-header',
  conversationBody: '.conversation-body',
  sideToolsResizer: '.side-tools-resizer',
  sideToolsPanel: '.thread-side-tools-panel',
  threadLayout: '.thread-layout',
  threadMain: '.thread-main',
  threadLogResizer: '.thread-log-resizer',
  threadLogPanel: '.thread-log-panel',
  taskTree: '.thread-subtask-pop',
  taskTreeToggle: '.thread-subtask-toggle',
  sidebarCarveout:
    '.app-shell > .sidebar-collapse-toggle-carveout:last-child',
};

const semanticClassTokens = {
  appShell: ['app-shell', 'sidebar-collapsed', 'with-bot-conversation-rail'],
  sidebarToggle: ['sidebar-collapse-toggle'],
  leftRail: ['left-rail'],
  conversationRail: [
    'bot-conversation-rail',
    'workspace-conversation-rail',
    'recent-conversation-rail',
  ],
  conversation: [
    'conversation',
    'thread-view',
    'with-side-tools',
    'side-tools-resizing',
  ],
  conversationHeader: ['conversation-header'],
  conversationBody: ['conversation-body'],
  sideToolsResizer: ['side-tools-resizer'],
  sideToolsPanel: [
    'thread-side-tools-panel',
    'is-picker-active',
    'is-files-active',
    'is-tasks-active',
    'is-chat-active',
    'is-browser-active',
    'is-terminal-active',
    'is-capsule-active',
  ],
  threadLayout: [
    'thread-layout',
    'with-inspector-panel',
    'with-log-panel',
    'log-panel-docked',
    'log-panel-overlay',
    'log-panel-resizing',
  ],
  threadMain: ['thread-main'],
  threadLogResizer: ['thread-log-resizer'],
  threadLogPanel: ['thread-log-panel'],
  taskTree: ['thread-subtask-pop', 'is-compact-open'],
  taskTreeToggle: ['thread-subtask-toggle', 'is-open'],
  sidebarCarveout: [
    'sidebar-collapse-toggle',
    'sidebar-collapse-toggle-carveout',
  ],
};

const trackedAttributeNames = [
  'aria-expanded',
  'aria-orientation',
  'aria-pressed',
  'aria-valuemax',
  'aria-valuemin',
  'aria-valuenow',
  'role',
];
const dynamicContentHeightKeys = ['taskTree'];

function parseArgs(argv) {
  let mode = 'compare';
  let cdpEndpoint = process.env.GARYX_LAYOUT_CDP_ENDPOINT || defaultCdpEndpoint;
  for (const arg of argv) {
    if (arg === '--write') {
      mode = 'write';
    } else if (arg === '--print') {
      mode = 'print';
    } else if (arg.startsWith('--cdp=')) {
      cdpEndpoint = arg.slice('--cdp='.length);
    } else if (arg === '--compare') {
      mode = 'compare';
    } else {
      throw new Error(`Unknown argument: ${arg}`);
    }
  }
  return { cdpEndpoint, mode };
}

async function settle(page) {
  await page.waitForTimeout(settleDelayMs);
  await page.evaluate(
    () =>
      new Promise((resolve) => {
        requestAnimationFrame(() => requestAnimationFrame(resolve));
      }),
  );
}

async function ensureSidebarOpen(page, open) {
  const toggle = page.locator(elementSelectors.sidebarToggle).first();
  const pressed = (await toggle.getAttribute('aria-pressed')) === 'true';
  if (open === !pressed) {
    return;
  }
  await page.locator(elementSelectors.sidebarCarveout).click();
  await page.waitForFunction(
    ({ selector, expectedPressed }) =>
      document.querySelector(selector)?.getAttribute('aria-pressed') ===
      expectedPressed,
    {
      selector: elementSelectors.sidebarToggle,
      expectedPressed: open ? 'false' : 'true',
    },
  );
  await settle(page);
}

async function ensureConversationRailOpen(page, open) {
  const rail = page.locator(elementSelectors.conversationRail).first();
  const present = (await rail.count()) > 0;
  if (present === open) {
    return;
  }
  if (open) {
    const recentAction = page.locator('.sidebar-nav .sidebar-action').last();
    await recentAction.click();
    await page.waitForSelector(elementSelectors.conversationRail);
  } else {
    await page.locator('.bot-conversation-collapse').first().click();
    await page.waitForSelector(elementSelectors.conversationRail, {
      state: 'detached',
    });
  }
  await settle(page);
}

async function ensureSideToolsOpen(page, open) {
  const toggle = page.locator(sideToolsToggleSelector).first();
  if ((await toggle.count()) === 0 || (await toggle.isDisabled())) {
    throw new Error(
      'The packaged oracle requires an active workspace thread with the side-tools toggle enabled.',
    );
  }
  const expanded = (await toggle.getAttribute('aria-expanded')) === 'true';
  const occupied = (await page.locator(elementSelectors.sideToolsPanel).count()) > 0;
  if (occupied && !expanded) {
    throw new Error(
      'The side-tools dock is capsule-only; restart the packaged app before recording the legacy oracle.',
    );
  }
  if (expanded === open) {
    return;
  }
  await toggle.click();
  await page.waitForFunction(
    ({ selector, expected }) =>
      document.querySelector(selector)?.getAttribute('aria-expanded') === expected,
    {
      selector: sideToolsToggleSelector,
      expected: open ? 'true' : 'false',
    },
  );
  await page.waitForSelector(elementSelectors.sideToolsPanel, {
    state: open ? 'attached' : 'detached',
  });
  await settle(page);
}

async function ensureThreadLogsOpen(page, open) {
  const toggle = page.locator('.conversation-header-action-logs').first();
  if ((await toggle.count()) === 0 || (await toggle.isDisabled())) {
    throw new Error(
      'The packaged oracle requires an active thread with the thread-logs toggle enabled.',
    );
  }
  const expanded = (await toggle.getAttribute('aria-expanded')) === 'true';
  if (expanded === open) {
    return;
  }
  await toggle.click();
  await page.waitForFunction(
    ({ selector, expected }) =>
      document.querySelector(selector)?.getAttribute('aria-expanded') === expected,
    {
      selector: '.conversation-header-action-logs',
      expected: open ? 'true' : 'false',
    },
  );
  await page.waitForSelector(elementSelectors.threadLogPanel, {
    state: open ? 'attached' : 'detached',
  });
  await settle(page);
}

async function resetPanels(page) {
  const threadInfoToggle = page.locator(
    '.thread-info-shell.is-open button.conversation-header-action-icon',
  );
  if ((await threadInfoToggle.count()) > 0) {
    await threadInfoToggle.click();
  }
  await ensureThreadLogsOpen(page, false);
  await ensureSideToolsOpen(page, false);
  await ensureConversationRailOpen(page, false);
  await ensureSidebarOpen(page, true);
}

async function requireTaskTree(page, presentation) {
  const taskTree = page.locator(elementSelectors.taskTree).first();
  if ((await taskTree.count()) === 0) {
    throw new Error(
      'The packaged oracle requires an active thread with a task forest so task-tree layout is captured.',
    );
  }
  if (
    presentation === 'overlay' &&
    (await page.locator(elementSelectors.taskTreeToggle).count()) === 0
  ) {
    throw new Error(
      'The Recent-rail oracle scenario requires the task tree to use its compact overlay presentation.',
    );
  }
}

async function persistedThreadLogsPanelWidth(page) {
  return page.evaluate(async () => {
    const state = await window.garyxDesktop.getState();
    return state.settings.threadLogsPanelWidth;
  });
}

async function setThreadLogsPanelWidth(page, targetWidth) {
  const resizer = page.locator(elementSelectors.threadLogResizer).first();
  if ((await resizer.count()) === 0) {
    throw new Error('Open thread logs before setting the canonical oracle width.');
  }

  const currentWidth = Number(await resizer.getAttribute('aria-valuenow'));
  const minWidth = Number(await resizer.getAttribute('aria-valuemin'));
  const maxWidth = Number(await resizer.getAttribute('aria-valuemax'));
  assert.ok(Number.isFinite(currentWidth), 'thread-log width must be numeric');
  assert.ok(
    targetWidth >= minWidth && targetWidth <= maxWidth,
    `thread-log width ${targetWidth} must fit ${minWidth}-${maxWidth}`,
  );
  if (currentWidth === targetWidth) {
    return;
  }

  const box = await resizer.boundingBox();
  assert.ok(box, 'thread-log resizer must be measurable');
  const startX = box.x + box.width / 2;
  const startY = box.y + Math.min(box.height / 2, 120);
  await page.mouse.move(startX, startY);
  await page.mouse.down();
  await page.waitForFunction(() => document.body.style.cursor === 'col-resize');
  await page.mouse.move(startX + currentWidth - targetWidth, startY, {
    steps: 8,
  });
  await page.mouse.up();
  await page.waitForFunction(
    ({ selector, expected }) =>
      document.querySelector(selector)?.getAttribute('aria-valuenow') ===
      String(expected),
    { selector: elementSelectors.threadLogResizer, expected: targetWidth },
  );
  await page.waitForFunction(
    async (expected) =>
      (await window.garyxDesktop.getState()).settings.threadLogsPanelWidth ===
      expected,
    targetWidth,
  );
  await settle(page);
}

async function restorePackagedState(page, originalThreadLogsPanelWidth) {
  await page.evaluate(async (width) => {
    const state = await window.garyxDesktop.getState();
    if (state.settings.threadLogsPanelWidth !== width) {
      await window.garyxDesktop.saveSettings({
        ...state.settings,
        threadLogsPanelWidth: width,
      });
    }
  }, originalThreadLogsPanelWidth);
  await page.reload({ waitUntil: 'domcontentloaded' });
  await page.waitForSelector('.app-shell');
  await page.waitForSelector('.thread-layout');
  await settle(page);
  await resetPanels(page);
}

async function captureScenario(page, name) {
  return page.evaluate(
    ({
      classTokenAllowlist,
      dynamicHeightKeys,
      elements,
      name: scenarioName,
      trackedAttributes,
    }) => {
      const round = (value) => Math.round(value * 100) / 100;
      const attrRecord = (node) => {
        const values = [];
        for (const name of trackedAttributes) {
          const value = node.getAttribute(name);
          if (value !== null) {
            values.push([name, value]);
          }
        }
        return Object.fromEntries(values);
      };
      const measure = (key, selector) => {
        const node = document.querySelector(selector);
        if (!(node instanceof HTMLElement)) {
          return null;
        }
        const rect = node.getBoundingClientRect();
        const computed = getComputedStyle(node);
        const transformScaleY =
          computed.transform === 'none'
            ? 1
            : new DOMMatrixReadOnly(computed.transform).m22;
        const transformMovesYWithDynamicHeight =
          key === 'taskTree' &&
          Math.abs(transformScaleY - 1) > Number.EPSILON;
        const allowed = new Set(classTokenAllowlist[key] || []);
        const classTokens = String(node.className)
          .split(/\s+/)
          .filter((token) => token && allowed.has(token));
        return {
          rect: {
            x: round(rect.x),
            y: transformMovesYWithDynamicHeight ? 'dynamic' : round(rect.y),
            width: round(rect.width),
            height: dynamicHeightKeys.includes(key) ? 'dynamic' : round(rect.height),
          },
          classTokens,
          attributes: attrRecord(node),
          computed: {
            display: computed.display,
            visibility: computed.visibility,
            position: computed.position,
            pointerEvents: computed.pointerEvents,
            appRegion: computed.getPropertyValue('-webkit-app-region').trim(),
            gridTemplateColumns: computed.gridTemplateColumns,
            gridTemplateRows: computed.gridTemplateRows,
          },
        };
      };

      const shell = document.querySelector('.app-shell');
      const sidebarToggle = document.querySelector(
        '.app-shell > button.sidebar-collapse-toggle',
      );
      const conversationRail = document.querySelector(elements.conversationRail);
      const sideToolsToggle = document.querySelector(
        '.conversation-header-actions > button.conversation-header-action-icon',
      );
      const logsToggle = document.querySelector(
        '.conversation-header-action-logs',
      );
      const sideToolsPanel = document.querySelector(elements.sideToolsPanel);
      const threadLayout = document.querySelector(elements.threadLayout);
      const taskTree = document.querySelector(elements.taskTree);
      const taskTreeToggle = document.querySelector(elements.taskTreeToggle);
      const railKind = conversationRail?.classList.contains(
        'recent-conversation-rail',
      )
        ? 'recent'
        : conversationRail?.classList.contains('workspace-conversation-rail')
          ? 'workspace'
          : conversationRail
            ? 'bot'
            : 'closed';
      const sidebarCollapsed = shell?.classList.contains('sidebar-collapsed') ?? true;
      const logsOpen = logsToggle?.getAttribute('aria-expanded') === 'true';
      const sideToolsIntent =
        sideToolsToggle?.getAttribute('aria-expanded') === 'true';
      const taskTreePresentation = !taskTree
        ? 'absent'
        : taskTreeToggle
          ? taskTreeToggle.getAttribute('aria-expanded') === 'true'
            ? 'overlay-open'
            : 'overlay-closed'
          : 'docked';

      return {
        name: scenarioName,
        viewport: {
          width: window.innerWidth,
          height: window.innerHeight,
          devicePixelRatio: window.devicePixelRatio,
        },
        desiredOccupancy: {
          globalSidebar: !sidebarCollapsed,
          conversationRail: railKind !== 'closed',
          sideTools: Boolean(sideToolsPanel),
          threadLogs: logsOpen,
        },
        legacyControlSignals: {
          sidebarAriaPressed: sidebarToggle?.getAttribute('aria-pressed') ?? null,
          conversationRailKind: railKind,
          sideToolsAriaExpanded:
            sideToolsToggle?.getAttribute('aria-expanded') ?? null,
          sideToolsOccupied: Boolean(sideToolsPanel),
          threadLogsAriaExpanded:
            logsToggle?.getAttribute('aria-expanded') ?? null,
        },
        presentation: {
          globalSidebar: sidebarCollapsed ? 'collapsed' : 'expanded',
          conversationRail: railKind,
          sideTools: sideToolsPanel ? 'docked' : 'closed',
          threadLogs: !logsOpen
            ? 'closed'
            : threadLayout?.classList.contains('log-panel-docked')
              ? 'docked'
              : 'overlay',
          taskTree: taskTreePresentation,
        },
        elements: Object.fromEntries(
          Object.entries(elements).map(([key, selector]) => [
            key,
            measure(key, selector),
          ]),
        ),
      };
    },
    {
      classTokenAllowlist: semanticClassTokens,
      dynamicHeightKeys: dynamicContentHeightKeys,
      elements: elementSelectors,
      name,
      trackedAttributes: trackedAttributeNames,
    },
  );
}

async function captureOracle(page) {
  await page.waitForSelector('.app-shell');
  await page.waitForSelector('.thread-layout');
  await settle(page);

  const viewport = await page.evaluate(() => ({
    width: window.innerWidth,
    height: window.innerHeight,
  }));
  assert.deepEqual(
    viewport,
    expectedViewport,
    'restart the packaged app before capture so the native 1480x940 legacy baseline is authoritative',
  );
  const originalThreadLogsPanelWidth =
    await persistedThreadLogsPanelWidth(page);
  assert.ok(
    Number.isFinite(originalThreadLogsPanelWidth),
    'persisted thread-log width must be numeric',
  );

  try {
    const scenarios = [];
    await resetPanels(page);
    await ensureThreadLogsOpen(page, true);
    await setThreadLogsPanelWidth(page, defaultThreadLogsPanelWidth);
    await ensureThreadLogsOpen(page, false);
    await requireTaskTree(page, 'docked');
    scenarios.push(await captureScenario(page, 'baseline'));

    await ensureSidebarOpen(page, false);
    scenarios.push(await captureScenario(page, 'sidebar-collapsed'));
    await ensureSidebarOpen(page, true);

    await ensureSideToolsOpen(page, true);
    scenarios.push(await captureScenario(page, 'side-tools'));
    await ensureSideToolsOpen(page, false);

    await ensureThreadLogsOpen(page, true);
    scenarios.push(await captureScenario(page, 'thread-logs'));
    await ensureThreadLogsOpen(page, false);

    await ensureConversationRailOpen(page, true);
    await requireTaskTree(page, 'overlay');
    scenarios.push(await captureScenario(page, 'recent-rail'));

    await ensureSideToolsOpen(page, true);
    scenarios.push(await captureScenario(page, 'recent-rail-side-tools'));
    await ensureSideToolsOpen(page, false);

    await ensureThreadLogsOpen(page, true);
    scenarios.push(await captureScenario(page, 'recent-rail-thread-logs'));

    await ensureThreadLogsOpen(page, false);
    await ensureConversationRailOpen(page, false);
    await ensureThreadLogsOpen(page, true);
    await setThreadLogsPanelWidth(page, overlayThreadLogsPanelWidth);
    await ensureThreadLogsOpen(page, false);
    await ensureConversationRailOpen(page, true);
    await ensureThreadLogsOpen(page, true);
    scenarios.push(
      await captureScenario(page, 'recent-rail-thread-logs-overlay'),
    );

    return {
      schemaVersion: 1,
      policy: 'legacy',
      capture: 'packaged-cdp-normalized-structure',
      scenarios,
    };
  } finally {
    await restorePackagedState(page, originalThreadLogsPanelWidth);
  }
}

async function main() {
  const { cdpEndpoint, mode } = parseArgs(process.argv.slice(2));
  const browser = await chromium.connectOverCDP(cdpEndpoint);
  try {
    const pages = browser.contexts().flatMap((context) => context.pages());
    const page = pages.find((candidate) => candidate.url().includes('index.html'));
    if (!page) {
      throw new Error(`No Garyx packaged renderer found at ${cdpEndpoint}`);
    }
    const actual = await captureOracle(page);
    const serialized = `${JSON.stringify(actual, null, 2)}\n`;
    if (mode === 'print') {
      process.stdout.write(serialized);
      return;
    }
    if (mode === 'write') {
      await mkdir(path.dirname(fixturePath), { recursive: true });
      await writeFile(fixturePath, serialized, 'utf8');
      console.log(`Wrote ${path.relative(projectDir, fixturePath)}`);
      return;
    }
    const expected = JSON.parse(await readFile(fixturePath, 'utf8'));
    assert.deepEqual(actual, expected);
    console.log(
      `Legacy horizontal layout oracle matched ${actual.scenarios.length} packaged scenarios.`,
    );
  } finally {
    await browser.close();
  }
}

await main();
