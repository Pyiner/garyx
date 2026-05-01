import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { fileURLToPath } from 'node:url';
import { readFileSync } from 'node:fs';
import { mkdtemp, mkdir, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { WebSocketServer } from 'ws';

import { _electron as electron } from 'playwright';
import { buildDesktopElectronLaunchEnv } from './electron-launch-env.mjs';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const projectDir = path.resolve(scriptDir, '..');
const SCREENSHOT_PATH = path.resolve(
  projectDir,
  'test-artifacts/electron-smoke.png',
);
const THREAD_ID = 'thread::smoke-seed';
const THREAD_LABEL = 'Smoke Thread';
const SLASH_COMMAND_NAME = 'summary';
const TOKENS = [
  'GARYX_DESKTOP_QUEUE_PASS_ONE_20260306',
  'GARYX_DESKTOP_QUEUE_PASS_TWO_20260306',
];
const WARMUP_TOKEN = 'GARYX_DESKTOP_QUEUE_WARMUP_20260318';
const RUN_ID = Date.now().toString(36);
const STARTUP_FAILURE_BUDGET = new Map([
  ['/api/threads', 2],
  ['/api/sessions', 2],
  ['/api/channel-endpoints', 1],
  ['/api/configured-bots', 2],
  ['/api/automations', 1],
]);

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function unescapeJsString(value) {
  return value
    .replace(/\\n/g, '\n')
    .replace(/\\r/g, '\r')
    .replace(/\\t/g, '\t')
    .replace(/\\'/g, "'")
    .replace(/\\"/g, '"')
    .replace(/\\\\/g, '\\');
}

function loadZhTranslation(key) {
  const i18nSource = readFileSync(
    path.join(projectDir, 'src/renderer/src/i18n/index.tsx'),
    'utf8',
  );
  const escapedKey = escapeRegExp(key).replace(/'/g, "\\'");
  const match = i18nSource.match(
    new RegExp(`['"]${escapedKey}['"]\\s*:\\s*(['"])((?:\\\\.|(?!\\1).)*)\\1`),
  );
  return match ? unescapeJsString(match[2]) : key;
}

function smokeLabel(key) {
  return [key, loadZhTranslation(key)];
}

const SMOKE_TEXT = {
  newThread: smokeLabel('New Thread'),
  startNewThread: smokeLabel('Start a new thread'),
  resumeExistingSession: smokeLabel('Resume existing session'),
  send: smokeLabel('Send'),
  settings: smokeLabel('Settings'),
  language: smokeLabel('Language'),
  followUpsReady: smokeLabel('2 follow-ups ready'),
  tabs: {
    General: smokeLabel('General'),
    Gateway: smokeLabel('Gateway'),
    Provider: smokeLabel('Provider'),
    Channels: smokeLabel('Channels'),
  },
};

function oneOfExact(values) {
  return new RegExp(`^(?:${values.map(escapeRegExp).join('|')})$`);
}

function sleep(ms) {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

async function prepareIsolatedHome(gatewayUrl, homeDirOverride = null) {
  const homeDir = homeDirOverride || (await mkdtemp(path.join(os.tmpdir(), 'garyx-desktop-smoke-')));
  const userDataDir = path.join(homeDir, 'user-data');
  const workspaceDir = path.join(homeDir, 'workspace');
  const localBookmarkDir = path.join(homeDir, 'local-bookmark');
  await mkdir(userDataDir, { recursive: true });
  await mkdir(workspaceDir, { recursive: true });
  await mkdir(localBookmarkDir, { recursive: true });
  await writeFile(
    path.join(userDataDir, 'garyx-desktop-state.json'),
    JSON.stringify(
      {
        settings: {
          gatewayUrl,
          accountId: `smoke-${RUN_ID}`,
          fromId: `playwright-smoke-${RUN_ID}`,
          timeoutSeconds: 120,
        },
        workspaces: [
          {
            name: path.basename(localBookmarkDir),
            path: localBookmarkDir,
            kind: 'local',
            createdAt: new Date().toISOString(),
            updatedAt: new Date().toISOString(),
            available: true,
          },
        ],
        selectedWorkspacePath: localBookmarkDir,
        sessions: [],
      },
      null,
      2,
      ),
    'utf8',
  );
  return { homeDir, userDataDir, workspaceDir };
}

function tokenFromPrompt(prompt) {
  const match = prompt.match(/token\s+([A-Z0-9_:-]+)/i);
  return match?.[1] || `SMOKE_${Date.now()}`;
}

function buildToolScenario(interactionIndex, workspaceDir) {
  if (interactionIndex % 2 === 0) {
    const filePath = path.join(workspaceDir, 'docs', 'design.md');
    return {
      historyToolUseContent: {
        tool: 'Read',
        input: {
          file_path: filePath,
          limit: 334,
        },
      },
      historyToolResultContent: {
        result: `# Design Document\n\n${'line\n'.repeat(8)}`.trim(),
        text: '# Design Document',
      },
      streamToolUseMessage: {
        role: 'tool_use',
        content: {
          tool: 'Read',
          input: {
            file_path: filePath,
            limit: 334,
          },
        },
        toolUseId: `tool:read:${interactionIndex}`,
        toolName: 'Read',
      },
      streamToolResultMessage: {
        role: 'tool_result',
        content: {
          result: `# Design Document\n\n${'line\n'.repeat(8)}`.trim(),
          text: '# Design Document',
        },
        toolUseId: `tool:read:${interactionIndex}`,
        toolName: 'Read',
      },
    };
  }

  return {
    historyToolUseContent: {
      type: 'mcpToolCall',
      id: `mcp-${interactionIndex}`,
      status: 'in_progress',
      server: 'filesystem',
      tool: 'read_file',
      arguments: {
        path: path.join(workspaceDir, 'docs', 'brief.md'),
      },
    },
    historyToolResultContent: {
      type: 'mcpToolCall',
      id: `mcp-${interactionIndex}`,
      status: 'completed',
      server: 'filesystem',
      tool: 'read_file',
      arguments: {
        path: path.join(workspaceDir, 'docs', 'brief.md'),
      },
      result: 'Project brief contents',
    },
    streamToolUseMessage: {
      role: 'tool_use',
      content: {
        type: 'mcpToolCall',
        id: `mcp-${interactionIndex}`,
        status: 'in_progress',
        server: 'filesystem',
        tool: 'read_file',
        arguments: {
          path: path.join(workspaceDir, 'docs', 'brief.md'),
        },
      },
      toolUseId: `tool:mcp:${interactionIndex}`,
      toolName: 'mcp:filesystem:read_file',
    },
    streamToolResultMessage: {
      role: 'tool_result',
      content: {
        type: 'mcpToolCall',
        id: `mcp-${interactionIndex}`,
        status: 'completed',
        server: 'filesystem',
        tool: 'read_file',
        arguments: {
          path: path.join(workspaceDir, 'docs', 'brief.md'),
        },
        result: 'Project brief contents',
      },
      toolUseId: `tool:mcp:${interactionIndex}`,
      toolName: 'mcp:filesystem:read_file',
    },
  };
}

async function createMockGateway(workspaceDir) {
  const state = {
    offlineUntil: 0,
    nextStreamDisconnect: null,
    threadCreateRequests: [],
    sessions: [
      {
        thread_id: THREAD_ID,
        session_key: THREAD_ID,
        label: THREAD_LABEL,
        workspace_dir: workspaceDir,
        updated_at: '2026-03-07T03:00:00Z',
        created_at: '2026-03-07T02:58:00Z',
        message_count: 0,
        last_user_message: null,
        last_assistant_message: null,
        channel_bindings: [],
      },
    ],
    histories: {
      [THREAD_ID]: [],
    },
    commands: [
      {
        name: SLASH_COMMAND_NAME,
        description: 'Summarize current repo issues',
        prompt: 'Summarize the latest repo issues that need action.',
      },
    ],
  };

  function readJson(req) {
    return new Promise((resolve, reject) => {
      let raw = '';
      req.on('data', (chunk) => {
        raw += chunk;
      });
      req.on('end', () => {
        try {
          resolve(raw ? JSON.parse(raw) : {});
        } catch (error) {
          reject(error);
        }
      });
      req.on('error', reject);
    });
  }

  function writeJson(res, status, payload) {
    res.writeHead(status, {
      'content-type': 'application/json',
    });
    res.end(JSON.stringify(payload));
  }

  function ensureSession(sessionId) {
    return state.sessions.find((session) => session.thread_id === sessionId) || null;
  }

  function appendStreamHistory(sessionId, userText) {
    const session = ensureSession(sessionId);
    if (!session) {
      return null;
    }
    const assistantText = tokenFromPrompt(userText);
    const history = state.histories[sessionId] || [];
    const userIndex = history.length;
    const interactionIndex = Math.floor(userIndex / 4);
    const scenario = buildToolScenario(interactionIndex, workspaceDir);
    history.push({
      index: userIndex,
      role: 'user',
      kind: 'user_input',
      timestamp: new Date().toISOString(),
      content: userText,
    });
    history.push({
      index: userIndex + 1,
      role: 'tool_use',
      kind: 'tool_trace',
      timestamp: new Date().toISOString(),
      content: JSON.stringify(scenario.historyToolUseContent),
    });
    history.push({
      index: userIndex + 2,
      role: 'tool_result',
      kind: 'tool_trace',
      timestamp: new Date().toISOString(),
      content: JSON.stringify(scenario.historyToolResultContent),
    });
    history.push({
      index: userIndex + 3,
      role: 'assistant',
      kind: 'assistant_reply',
      timestamp: new Date().toISOString(),
      content: assistantText,
    });
    state.histories[sessionId] = history;
    session.updated_at = new Date().toISOString();
    session.message_count = history.length;
    session.last_user_message = userText;
    session.last_assistant_message = assistantText;
    return {
      runId: `run::${Date.now()}`,
      assistantText,
      scenario,
      sessionId,
    };
  }

  async function streamRunToSocket(socket, payload) {
    socket.send(
      JSON.stringify({
        type: 'accepted',
        runId: payload.runId,
        threadId: payload.sessionId,
      }),
    );
    await sleep(120);
    socket.send(
      JSON.stringify({
        type: 'tool_use',
        runId: payload.runId,
        threadId: payload.sessionId,
        message: payload.scenario.streamToolUseMessage,
      }),
    );
    await sleep(120);
    socket.send(
      JSON.stringify({
        type: 'tool_result',
        runId: payload.runId,
        threadId: payload.sessionId,
        message: payload.scenario.streamToolResultMessage,
      }),
    );
    await sleep(120);
    socket.send(
      JSON.stringify({
        type: 'assistant_delta',
        runId: payload.runId,
        threadId: payload.sessionId,
        delta: payload.assistantText,
      }),
    );
    if (state.nextStreamDisconnect) {
      state.offlineUntil = Date.now() + state.nextStreamDisconnect.offlineMs;
      state.nextStreamDisconnect = null;
      socket.close();
      return;
    }
    await sleep(60);
    socket.send(
      JSON.stringify({
        type: 'done',
        runId: payload.runId,
        threadId: payload.sessionId,
      }),
    );
  }

  const wsServer = new WebSocketServer({ noServer: true });
  wsServer.on('connection', (socket) => {
    socket.on('message', async (raw) => {
      let payload;
      try {
        payload = JSON.parse(String(raw));
      } catch {
        socket.send(JSON.stringify({ type: 'error', error: 'invalid websocket json payload' }));
        return;
      }
      const op = payload.op;
      if (op === 'start') {
        const sessionId = payload.threadId || payload.sessionKey || THREAD_ID;
        const session = ensureSession(sessionId);
        if (!session) {
          socket.send(JSON.stringify({ type: 'error', error: 'thread not found', threadId: sessionId }));
          return;
        }
        const streamPayload = appendStreamHistory(sessionId, String(payload.message || ''));
        if (!streamPayload) {
          socket.send(JSON.stringify({ type: 'error', error: 'thread not found', threadId: sessionId }));
          return;
        }
        await streamRunToSocket(socket, streamPayload);
        return;
      }
      if (op === 'input') {
        socket.send(
          JSON.stringify({
            type: 'stream_input',
            status: 'no_active_session',
            threadId: payload.threadId || THREAD_ID,
          }),
        );
        return;
      }
      if (op === 'interrupt') {
        socket.send(
          JSON.stringify({
            type: 'interrupt',
            status: 'ok',
            threadId: payload.threadId || THREAD_ID,
            abortedRuns: [],
          }),
        );
      }
    });
  });

  const server = createServer(async (req, res) => {
    const url = new URL(req.url || '/', 'http://127.0.0.1');
    const { pathname } = url;
    const offline = Date.now() < state.offlineUntil;

    if (offline) {
      return writeJson(res, 503, {
        error: 'gateway temporarily offline',
      });
    }

    const remainingFailures = STARTUP_FAILURE_BUDGET.get(pathname) || 0;
    if (req.method === 'GET' && remainingFailures > 0) {
      STARTUP_FAILURE_BUDGET.set(pathname, remainingFailures - 1);
      return writeJson(res, 503, {
        error: `startup warming ${pathname}`,
      });
    }

    if (req.method === 'GET' && pathname === '/api/chat/health') {
      return writeJson(res, 200, { status: 'ok', bridge_ready: true });
    }
    if (req.method === 'GET' && pathname === '/api/status') {
      return writeJson(res, 200, {
        status: 'running',
        sessions: { count: state.sessions.length },
        stream: { drops: 0, history_size: 0 },
        uptime_seconds: 5,
        version: '0.1.0',
      });
    }
    if (req.method === 'GET' && pathname === '/api/commands/shortcuts') {
      return writeJson(res, 200, {
        commands: state.commands,
      });
    }
    if (req.method === 'GET' && pathname === '/runtime') {
      const address = server.address();
      return writeJson(res, 200, {
        runtime: { uptime_seconds: 5, version: '0.1.0' },
        gateway: {
          host: '127.0.0.1',
          port: typeof address === 'object' && address ? address.port : 0,
        },
      });
    }
    if (req.method === 'GET' && pathname === '/api/settings') {
      return writeJson(res, 200, { config: {} });
    }
    if (req.method === 'GET' && (pathname === '/api/threads' || pathname === '/api/sessions')) {
      return writeJson(res, 200, {
        threads: state.sessions,
        sessions: state.sessions,
        count: state.sessions.length,
        total: state.sessions.length,
        limit: 100,
        offset: 0,
      });
    }
    if (req.method === 'GET' && pathname === '/api/channel-endpoints') {
      return writeJson(res, 200, { endpoints: [] });
    }
    if (req.method === 'GET' && pathname === '/api/configured-bots') {
      return writeJson(res, 200, { bots: [] });
    }
    if (req.method === 'GET' && pathname === '/api/bot-consoles') {
      return writeJson(res, 200, { bots: [] });
    }
    if (req.method === 'GET' && pathname === '/api/automations') {
      return writeJson(res, 200, { automations: [] });
    }
    if (
      req.method === 'GET' &&
      (pathname === '/api/threads/history' || pathname === '/api/sessions/history')
    ) {
      const sessionId =
        url.searchParams.get('threadId') ||
        url.searchParams.get('thread_id') ||
        url.searchParams.get('session_key') ||
        THREAD_ID;
      return writeJson(res, 200, {
        ok: Boolean(ensureSession(sessionId)),
        messages: state.histories[sessionId] || [],
      });
    }
    if (req.method === 'POST' && (pathname === '/api/threads' || pathname === '/api/sessions')) {
      const body = await readJson(req);
      state.threadCreateRequests.push(body);
      const now = new Date().toISOString();
      const threadId = `thread::smoke-${Date.now()}`;
      const session = {
        thread_id: threadId,
        session_key: threadId,
        label: body.label || 'Fresh Thread',
        workspace_dir: body.workspaceDir || workspaceDir,
        updated_at: now,
        created_at: now,
        message_count: 0,
        last_user_message: null,
        last_assistant_message: null,
        channel_bindings: [],
      };
      state.sessions.unshift(session);
      state.histories[session.thread_id] = [];
      return writeJson(res, 200, session);
    }
    if (req.method === 'POST' && pathname === '/api/chat') {
      const body = await readJson(req);
      const sessionId = body.threadId || body.sessionKey || THREAD_ID;
      const session = ensureSession(sessionId);
      if (!session) {
        return writeJson(res, 404, { error: 'thread not found' });
      }

      const userText = String(body.message || '');
      const assistantText = tokenFromPrompt(userText);
      const history = state.histories[sessionId] || [];
      const userIndex = history.length;
      history.push({
        index: userIndex,
        role: 'user',
        kind: 'user_input',
        timestamp: new Date().toISOString(),
        content: userText,
      });
      history.push({
        index: userIndex + 1,
        role: 'assistant',
        kind: 'assistant_reply',
        timestamp: new Date().toISOString(),
        content: assistantText,
      });
      state.histories[sessionId] = history;
      session.updated_at = new Date().toISOString();
      session.message_count = history.length;
      session.last_user_message = userText;
      session.last_assistant_message = assistantText;

      return writeJson(res, 200, {
        runId: `run::${Date.now()}`,
        threadId: sessionId,
        sessionKey: sessionId,
        response: assistantText,
      });
    }
    if (req.method === 'POST' && pathname === '/api/chat/stream') {
      const body = await readJson(req);
      const sessionId = body.threadId || body.sessionKey || THREAD_ID;
      const session = ensureSession(sessionId);
      if (!session) {
        return writeJson(res, 404, { error: 'thread not found' });
      }

      const userText = String(body.message || '');
      const assistantText = tokenFromPrompt(userText);
      const history = state.histories[sessionId] || [];
      const userIndex = history.length;
      const interactionIndex = Math.floor(userIndex / 4);
      const scenario = buildToolScenario(interactionIndex, workspaceDir);
      history.push({
        index: userIndex,
        role: 'user',
        kind: 'user_input',
        timestamp: new Date().toISOString(),
        content: userText,
      });
      history.push({
        index: userIndex + 1,
        role: 'tool_use',
        kind: 'tool_trace',
        timestamp: new Date().toISOString(),
        content: JSON.stringify(scenario.historyToolUseContent),
      });
      history.push({
        index: userIndex + 2,
        role: 'tool_result',
        kind: 'tool_trace',
        timestamp: new Date().toISOString(),
        content: JSON.stringify(scenario.historyToolResultContent),
      });
      history.push({
        index: userIndex + 3,
        role: 'assistant',
        kind: 'assistant_reply',
        timestamp: new Date().toISOString(),
        content: assistantText,
      });
      state.histories[sessionId] = history;
      session.updated_at = new Date().toISOString();
      session.message_count = history.length;
      session.last_user_message = userText;
      session.last_assistant_message = assistantText;

      const runId = `run::${Date.now()}`;
      const sendEvent = (eventName, payload) => {
        res.write(`event: ${eventName}\n`);
        res.write(`data: ${JSON.stringify(payload)}\n\n`);
      };

      res.writeHead(200, {
        'content-type': 'text/event-stream',
        'cache-control': 'no-cache',
        connection: 'keep-alive',
      });

      sendEvent('accepted', {
        runId,
        threadId: sessionId,
        sessionKey: sessionId,
      });
      await sleep(120);
      sendEvent('tool_use', {
        runId,
        threadId: sessionId,
        sessionKey: sessionId,
        message: scenario.streamToolUseMessage,
      });
      await sleep(120);
      sendEvent('tool_result', {
        runId,
        threadId: sessionId,
        sessionKey: sessionId,
        message: scenario.streamToolResultMessage,
      });
      await sleep(120);
      sendEvent('assistant_delta', {
        runId,
        threadId: sessionId,
        sessionKey: sessionId,
        delta: assistantText,
      });
      if (state.nextStreamDisconnect) {
        state.offlineUntil = Date.now() + state.nextStreamDisconnect.offlineMs;
        state.nextStreamDisconnect = null;
        res.destroy();
        return;
      }
      await sleep(60);
      sendEvent('done', {
        runId,
        threadId: sessionId,
        sessionKey: sessionId,
      });
      res.end();
      return;
    }
    if (req.method === 'POST' && pathname === '/api/chat/stream-input') {
      const body = await readJson(req);
      return writeJson(res, 200, {
        status: 'no_active_session',
        threadId: body.threadId || body.sessionKey || THREAD_ID,
        sessionKey: body.sessionKey || THREAD_ID,
      });
    }

    writeJson(res, 404, { error: 'not found' });
  });

  await new Promise((resolve) => {
    server.listen(0, '127.0.0.1', resolve);
  });

  server.on('upgrade', (req, socket, head) => {
    const url = new URL(req.url || '/', 'http://127.0.0.1');
    if (url.pathname !== '/api/chat/ws') {
      socket.destroy();
      return;
    }
    if (Date.now() < state.offlineUntil) {
      socket.destroy();
      return;
    }
    wsServer.handleUpgrade(req, socket, head, (ws) => {
      wsServer.emit('connection', ws, req);
    });
  });

  const address = server.address();
  const port = typeof address === 'object' && address ? address.port : 0;
  return {
    gatewayUrl: `http://127.0.0.1:${port}`,
    createdThreadRequests: () => [...state.threadCreateRequests],
    scheduleTransientDisconnect: ({ offlineMs = 3500 } = {}) => {
      state.nextStreamDisconnect = { offlineMs };
    },
    close: () =>
      new Promise((resolve, reject) => {
        wsServer.close(() => {
          server.close((error) => {
            if (error) {
              reject(error);
            } else {
              resolve();
            }
          });
        });
      }),
  };
}

async function main() {
  const homeDir = await mkdtemp(path.join(os.tmpdir(), 'garyx-desktop-smoke-home-'));
  const workspaceDir = path.join(homeDir, 'workspace');
  const gateway = await createMockGateway(workspaceDir);
  const { homeDir: isolatedHome, userDataDir } = await prepareIsolatedHome(
    gateway.gatewayUrl,
    homeDir,
  );
  const electronApp = await electron.launch({
    args: [projectDir],
    cwd: projectDir,
    env: buildDesktopElectronLaunchEnv({
      HOME: isolatedHome,
      GARYX_DESKTOP_USER_DATA_PATH: userDataDir,
    }),
  });

  try {
    const window = await electronApp.firstWindow();
    const pageErrors = [];
    let stage = 'startup';
    window.on('pageerror', (error) => {
      pageErrors.push(error?.stack || error?.message || String(error));
    });
    await window.waitForLoadState('domcontentloaded');
    await window.setViewportSize({ width: 1480, height: 940 });
    try {
      stage = 'wait-thread-list';
      await window.getByRole('button', { name: oneOfExact(SMOKE_TEXT.newThread) }).waitFor({
        timeout: 15000,
      });
      try {
        await window.locator('.thread-title').filter({ hasText: THREAD_LABEL }).first().waitFor({
          timeout: 6000,
        });
      } catch {
        await window.getByText(THREAD_LABEL, { exact: true }).first().waitFor({
          timeout: 6000,
        });
      }
      stage = 'selected-workspace-new-thread';
      await window.getByRole('button', { name: oneOfExact(SMOKE_TEXT.newThread) }).click();
      await window.getByText(oneOfExact(SMOKE_TEXT.startNewThread)).waitFor({
        timeout: 10000,
      });
      await window.getByText(oneOfExact(SMOKE_TEXT.resumeExistingSession)).waitFor({
        timeout: 10000,
      });

      stage = 'managed-workspace-new-thread';
      await window.getByRole('button', { name: oneOfExact(SMOKE_TEXT.newThread) }).click();
      await window.getByText(oneOfExact(SMOKE_TEXT.startNewThread)).waitFor({
        timeout: 10000,
      });
      await window.getByText(oneOfExact(SMOKE_TEXT.resumeExistingSession)).waitFor({
        timeout: 10000,
      });

      stage = 'warmup-send';
      const composer = window.locator('.composer textarea');
      stage = 'slash-command-palette';
      await composer.fill('/');
      await window.locator('[data-testid="slash-command-panel"]').waitFor({
        timeout: 10000,
      });
      await window.locator(`[data-testid="slash-command-option-${SLASH_COMMAND_NAME}"]`).waitFor({
        timeout: 10000,
      });
      await composer.press('Enter');
      await window.waitForFunction(
        ([selector, expectedValue]) => {
          const element = document.querySelector(selector);
          return element instanceof HTMLTextAreaElement && element.value === expectedValue;
        },
        ['.composer textarea', `/${SLASH_COMMAND_NAME} `],
        { timeout: 10000 },
      );

      stage = 'warmup-send';
      await composer.fill(`Return exactly the token ${WARMUP_TOKEN} and nothing else.`);
      await window.getByRole('button', { name: oneOfExact(SMOKE_TEXT.send) }).click();
      await window.locator('.tool-trace').first().waitFor({ timeout: 20000 });
      stage = 'verify-new-thread-workspace-path';
      const createRequests = gateway.createdThreadRequests();
      const expectedWorkspacePath = path.join(isolatedHome, 'workspace');
      assert.ok(createRequests.length >= 1, 'expected a new thread create request');
      const firstCreateRequest = createRequests[0];
      assert.equal(
        firstCreateRequest.workspaceDir,
        expectedWorkspacePath,
        `new thread should use the active workspace path, got ${JSON.stringify(firstCreateRequest)}`,
      );
      assert.equal(
        Object.prototype.hasOwnProperty.call(firstCreateRequest, 'workspacePath'),
        false,
        `new thread request should send only workspaceDir, got ${JSON.stringify(firstCreateRequest)}`,
      );
      stage = 'reload-preserves-thread-route';
      await window.waitForFunction(
        () => window.location.hash.startsWith('#/thread/'),
        null,
        { timeout: 10000 },
      );
      const routeHashBeforeReload = await window.evaluate(() => window.location.hash);
      await window.reload({ waitUntil: 'domcontentloaded' });
      await window.locator('.app-shell').waitFor({ timeout: 15000 });
      await window.waitForFunction(
        (expectedHash) => window.location.hash === expectedHash,
        routeHashBeforeReload,
        { timeout: 10000 },
      );
      await window
        .locator('.message-bubble.assistant p')
        .filter({ hasText: WARMUP_TOKEN })
        .first()
        .waitFor({ timeout: 20000 });
      await window.getByRole('button', { name: oneOfExact(SMOKE_TEXT.send) }).waitFor({
        timeout: 15000,
      });
      stage = 'queue-followups';
      await composer.fill(`Return exactly the token ${TOKENS[0]} and nothing else.`);
      await composer.press('Enter');
      await window
        .locator('.message-bubble.user')
        .filter({ hasText: TOKENS[0] })
        .first()
        .waitFor({ timeout: 10000 });
      await composer.fill(`Return exactly the token ${TOKENS[1]} and nothing else.`);
      await composer.press('Enter');
      await window.getByText(oneOfExact(SMOKE_TEXT.followUpsReady)).waitFor({ timeout: 3000 }).catch(() => {});
      await window
        .locator('.message-bubble.assistant p')
        .filter({ hasText: TOKENS[0] })
        .first()
        .waitFor({ timeout: 20000 });
      await window
        .locator('.message-bubble.assistant p')
        .filter({ hasText: TOKENS[1] })
        .first()
        .waitFor({ timeout: 20000 });

      stage = 'verify-tool-traces';
      const assistantTexts = await window
        .locator('.message-bubble.assistant p')
        .allTextContents();
      const toolTraceCount = await window.locator('.tool-trace').count();
      const toolTraceTexts = await window.locator('.tool-trace').allTextContents();
      for (const token of TOKENS) {
        assert.ok(
          assistantTexts.some((text) => text.includes(token)),
          `assistant replies did not contain queue token ${token}, got: ${assistantTexts.join(' | ') || '<empty>'}`,
        );
      }
      assert.ok(toolTraceCount >= 2, `expected tool traces to be visible, got ${toolTraceCount}`);
      assert.ok(
        toolTraceTexts.some((text) => text.includes('Read 334 lines')),
        `expected Claude-style Read tool trace, got: ${toolTraceTexts.join(' | ') || '<empty>'}`,
      );
      assert.ok(
        toolTraceTexts.some((text) => {
          return (
            (text.includes('Read file') || text.includes('MCP')) &&
            text.includes('filesystem') &&
            text.includes('docs/brief.md')
          );
        }),
        `expected Codex-style MCP tool trace, got: ${toolTraceTexts.join(' | ') || '<empty>'}`,
      );

      await window.getByRole('button', { name: oneOfExact(SMOKE_TEXT.send) }).waitFor({ timeout: 15000 });
      await window.waitForTimeout(300);

      stage = 'settings-navigation';
      await window.getByRole('button', { name: oneOfExact(SMOKE_TEXT.settings) }).click();
      for (const [tab, labels] of Object.entries(SMOKE_TEXT.tabs)) {
        await window.getByRole('button', { name: oneOfExact(labels) }).click();
        await window
          .locator('.settings-page-header .settings-tab-title')
          .filter({ hasText: oneOfExact(labels) })
          .waitFor({ timeout: 10000 });
        if (tab === 'Gateway') {
          await window.getByText(oneOfExact(SMOKE_TEXT.language)).first().waitFor({
            timeout: 10000,
          });
        }
      }

      assert.deepEqual(
        pageErrors,
        [],
        `settings navigation emitted renderer errors: ${pageErrors.join('\n\n')}`,
      );

      stage = 'success-screenshot';
      await mkdir(path.dirname(SCREENSHOT_PATH), { recursive: true });
      await window.locator('.app-shell').screenshot({ path: SCREENSHOT_PATH });

      console.log(
        JSON.stringify(
          {
            ok: true,
            tokens: TOKENS,
            screenshot: SCREENSHOT_PATH,
            isolatedHome,
            gatewayUrl: gateway.gatewayUrl,
            assistantTexts,
            toolTraceCount,
            toolTraceTexts,
          },
          null,
          2,
        ),
      );
    } catch (error) {
      await mkdir(path.dirname(SCREENSHOT_PATH), { recursive: true });
      let screenshotError = null;
      await window
        .locator('.app-shell')
        .screenshot({ path: SCREENSHOT_PATH, timeout: 5000 })
        .catch(async (appShellError) => {
          screenshotError = appShellError;
          try {
            await window.screenshot({ path: SCREENSHOT_PATH, timeout: 5000 });
            screenshotError = null;
          } catch (pageError) {
            screenshotError = pageError;
          }
        });
      const bodyText = await window.locator('body').textContent().catch(() => '');
      console.error(
        JSON.stringify(
          {
            ok: false,
            stage,
            screenshot: SCREENSHOT_PATH,
            isolatedHome,
            bodyText,
            error: error?.stack || error?.message || String(error),
            screenshotError:
              screenshotError?.stack || screenshotError?.message || null,
          },
          null,
          2,
        ),
      );
      throw error;
    }
  } finally {
    await electronApp.close();
    await gateway.close();
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
