#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process";
import { createServer } from "node:net";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const projectDir = path.resolve(scriptDir, "..");
const DEFAULT_FRAMES = 12;
const DEFAULT_ROWS = 40;
const ACTIVE_THREAD_ID = "__garyx_new_thread_draft__";
const BACKGROUND_THREAD_ID = "thread::render-oracle-background";

function parsePositiveInteger(flag, fallback) {
  const index = process.argv.indexOf(flag);
  if (index < 0) {
    return fallback;
  }
  const value = Number(process.argv[index + 1]);
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new Error(`${flag} must be a positive integer`);
  }
  return value;
}

function parseExpectation() {
  const index = process.argv.indexOf("--expect");
  if (index < 0) {
    return null;
  }
  const value = process.argv[index + 1];
  if (value !== "baseline" && value !== "optimized") {
    throw new Error("--expect must be baseline or optimized");
  }
  return value;
}

function gatewayAuthToken() {
  const result = spawnSync("garyx", ["gateway", "token", "--json"], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  if (result.error || result.status !== 0) {
    throw new Error("Could not read the local gateway token for the isolated oracle app");
  }
  const payload = JSON.parse(result.stdout);
  const token = payload.token || payload.auth_token || payload.gatewayAuthToken;
  if (typeof token !== "string" || !token.trim()) {
    throw new Error("garyx gateway token --json returned no token");
  }
  return token.trim();
}

async function freePort() {
  const server = createServer();
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  const port = typeof address === "object" && address ? address.port : 0;
  await new Promise((resolve) => server.close(resolve));
  if (!port) {
    throw new Error("Could not reserve a CDP port");
  }
  return port;
}

async function waitForCdp(port, child, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`electron-vite exited before CDP was ready (${child.exitCode})`);
    }
    try {
      const response = await fetch(`http://127.0.0.1:${port}/json/version`);
      if (response.ok) {
        return;
      }
    } catch {
      // The isolated Electron process is still starting.
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`CDP did not become ready on port ${port}`);
}

async function stopProcessGroup(child) {
  if (!child.pid || child.exitCode !== null) {
    return;
  }
  try {
    process.kill(-child.pid, "SIGTERM");
  } catch {
    child.kill("SIGTERM");
  }
  const exited = await Promise.race([
    new Promise((resolve) => child.once("exit", () => resolve(true))),
    new Promise((resolve) => setTimeout(() => resolve(false), 3_000)),
  ]);
  if (exited) {
    return;
  }
  try {
    process.kill(-child.pid, "SIGKILL");
  } catch {
    child.kill("SIGKILL");
  }
}

function assertExpectation(result, expectation) {
  if (!expectation) {
    return;
  }
  if (expectation === "baseline") {
    if (result.background.appShellRenders < result.frames) {
      throw new Error("baseline did not reproduce one AppShell render per background frame");
    }
    if (result.active.stableRowRenders < (result.rows - 1) * result.frames) {
      throw new Error("baseline did not reproduce full stable-row rerendering");
    }
    return;
  }
  if (result.background.appShellRenders !== 0) {
    throw new Error("background transcript frames still rendered AppShell");
  }
  if (result.active.stableRowRenders !== 0) {
    throw new Error("unchanged transcript rows still rendered during active streaming");
  }
}

async function runOracle(page, frames, rows) {
  const rendererUrl = page.url().split("#", 1)[0];
  await page.goto(`${rendererUrl}#/new`);
  await page.waitForFunction(() => Boolean(window.__garyxGatewayMirror), null, {
    timeout: 30_000,
  });
  await page.waitForSelector(".thread-main.new-thread-centered", {
    timeout: 30_000,
  });

  return page.evaluate(
    async ({ activeThreadId, backgroundThreadId, frameCount, rowCount }) => {
      const mirror = window.__garyxGatewayMirror;
      if (!mirror) {
        throw new Error("dev GatewayMirror handle is unavailable");
      }

      const counts = {
        AppShell: 0,
        ThreadPage: 0,
        "ThreadPage.row": 0,
        rowKeys: {},
      };
      window.__garyxTranscriptRenderProbe = {
        record(surface, key) {
          counts[surface] += 1;
          if (surface === "ThreadPage.row" && key) {
            counts.rowKeys[key] = (counts.rowKeys[key] || 0) + 1;
          }
        },
      };

      const nextPaint = () =>
        new Promise((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(resolve)),
        );
      const reset = () => {
        counts.AppShell = 0;
        counts.ThreadPage = 0;
        counts["ThreadPage.row"] = 0;
        counts.rowKeys = {};
      };
      const snapshot = () => ({
        appShellRenders: counts.AppShell,
        threadPageRenders: counts.ThreadPage,
        rowRenders: counts["ThreadPage.row"],
        rowKeys: { ...counts.rowKeys },
      });

      await nextPaint();
      reset();
      for (let frame = 1; frame <= frameCount; frame += 1) {
        mirror.syncThreadUiMessages(backgroundThreadId, [
          {
            id: `background-${frame}`,
            seq: frame,
            role: "assistant",
            text: `background frame ${frame}`,
            timestamp: "2026-01-01T00:00:00.000Z",
          },
        ]);
        await nextPaint();
      }
      const background = snapshot();

      const events = [];
      const renderRows = [];
      let committedSeq = 0;
      let toolRows = 0;
      let tailKey = null;
      for (let index = 0; index < rowCount; index += 1) {
        const userSeq = ++committedSeq;
        const userId = `oracle-user:${userSeq - 1}`;
        events.push({
          type: "committed_message",
          runId: "run-oracle-seed",
          threadId: activeThreadId,
          seq: userSeq,
          message: {
            id: userId,
            seq: userSeq,
            role: "user",
            text: `question ${index}`,
            content: `question ${index}`,
            timestamp: "2026-01-01T00:00:00.000Z",
          },
        });

        const activity = [];
        if (index % 5 === 0 && index < rowCount - 1) {
          toolRows += 1;
          const assistantSeq = ++committedSeq;
          const toolUseSeq = ++committedSeq;
          const toolResultSeq = ++committedSeq;
          const assistantId = `oracle-assistant:${assistantSeq - 1}`;
          const toolUseId = `oracle-tool-use:${toolUseSeq - 1}`;
          const toolResultId = `oracle-tool-result:${toolResultSeq - 1}`;
          const callId = `oracle-call-${index}`;
          const groupId = `oracle-tool-group-${index}`;
          events.push(
            {
              type: "committed_message",
              runId: "run-oracle-seed",
              threadId: activeThreadId,
              seq: assistantSeq,
              message: {
                id: assistantId,
                seq: assistantSeq,
                role: "assistant",
                text: `checking ${index}`,
                content: `checking ${index}`,
                timestamp: "2026-01-01T00:00:01.000Z",
              },
            },
            {
              type: "committed_message",
              runId: "run-oracle-seed",
              threadId: activeThreadId,
              seq: toolUseSeq,
              message: {
                id: toolUseId,
                seq: toolUseSeq,
                role: "tool_use",
                text: "oracle command",
                content: { command: "printf oracle" },
                toolUseId: callId,
                toolName: "commandExecution",
                timestamp: "2026-01-01T00:00:02.000Z",
              },
            },
            {
              type: "committed_message",
              runId: "run-oracle-seed",
              threadId: activeThreadId,
              seq: toolResultSeq,
              message: {
                id: toolResultId,
                seq: toolResultSeq,
                role: "tool_result",
                text: "oracle",
                content: { aggregatedOutput: "oracle" },
                toolUseId: callId,
                toolName: "commandExecution",
                timestamp: "2026-01-01T00:00:03.000Z",
              },
            },
          );
          activity.push({
            kind: "step",
            id: `oracle-step-${index}`,
            steps: [
              {
                kind: "assistant_message",
                id: `oracle-assistant-step-${index}`,
                message: { id: assistantId, seq: assistantSeq, role: "assistant" },
                streaming: false,
              },
              {
                kind: "tool_group",
                id: groupId,
                status: "completed",
                entries: [
                  {
                    id: `oracle-tool-entry-${index}`,
                    tool_use_id: callId,
                    status: "completed",
                    tool_use: { id: toolUseId, seq: toolUseSeq, role: "tool_use" },
                    tool_result: {
                      id: toolResultId,
                      seq: toolResultSeq,
                      role: "tool_result",
                    },
                    projection: {
                      tool_name: "commandExecution",
                      kind: "command",
                      visibility: "normal",
                      call: {
                        root: "content",
                        path: ["command"],
                        format: "code",
                        label: "command",
                      },
                      result: {
                        root: "content",
                        path: ["aggregatedOutput"],
                        format: "code",
                        label: "output",
                      },
                      status: "completed",
                      exit_code: 0,
                      duration_ms: 1,
                    },
                  },
                ],
                started_at: "2026-01-01T00:00:02.000Z",
                finished_at: "2026-01-01T00:00:03.000Z",
              },
            ],
            final_message: null,
            running: false,
            started_at: "2026-01-01T00:00:01.000Z",
            finished_at: "2026-01-01T00:00:03.000Z",
          });
        } else {
          const assistantSeq = ++committedSeq;
          const assistantId = `oracle-assistant:${assistantSeq - 1}`;
          events.push({
            type: "committed_message",
            runId: "run-oracle-seed",
            threadId: activeThreadId,
            seq: assistantSeq,
            message: {
              id: assistantId,
              seq: assistantSeq,
              role: "assistant",
              text: `answer ${index}`,
              content: `answer ${index}`,
              timestamp: "2026-01-01T00:00:01.000Z",
            },
          });
          activity.push({
            kind: "assistant_reply",
            id: `oracle-reply-${index}`,
            message: { id: assistantId, seq: assistantSeq, role: "assistant" },
            streaming: false,
          });
        }
        renderRows.push({
          kind: "user_turn",
          id: `oracle-turn-${index}`,
          user: { id: userId, seq: userSeq, role: "user" },
          activity,
          capsule_cards: [],
          started_at: "2026-01-01T00:00:00.000Z",
          finished_at: "2026-01-01T00:00:03.000Z",
        });
        if (index === rowCount - 1) {
          tailKey = `user-turn:${userId}`;
        }
      }
      const renderState = {
        based_on_seq: committedSeq,
        rows: renderRows,
        tailActivity: "none",
        activeToolGroupId: null,
        progress_locus: "none",
        filtered_placeholders: [],
      };
      mirror.ingest({
        type: "thread_render_frame",
        threadId: activeThreadId,
        events,
        renderState,
      });
      await nextPaint();
      const mountedRows = document.querySelectorAll(".messages-item").length;
      if (mountedRows !== rowCount) {
        const activeSnapshot = mirror.getThreadSnapshot(activeThreadId);
        throw new Error(
          `expected ${rowCount} mounted rows, found ${mountedRows}; ` +
            `mirror messages=${activeSnapshot.messages.length}, ` +
            `render rows=${activeSnapshot.renderState?.rows.length || 0}, ` +
            `hash=${window.location.hash}`,
        );
      }

      reset();
      const tailRow = renderRows[renderRows.length - 1];
      for (let frame = 1; frame <= frameCount; frame += 1) {
        const seq = committedSeq + frame;
        const id = `oracle-stream:${seq - 1}`;
        const nextTailRow = {
          ...tailRow,
          activity: [
            {
              kind: "assistant_reply",
              message: { id, seq, role: "assistant" },
            },
          ],
          finished_at: null,
        };
        const nextRows = structuredClone([
          ...renderRows.slice(0, -1),
          nextTailRow,
        ]);
        mirror.ingest({
          type: "thread_render_frame",
          threadId: activeThreadId,
          events: [
            {
              type: "committed_message",
              runId: "run-oracle-stream",
              threadId: activeThreadId,
              seq,
              message: {
                id,
                seq,
                role: "assistant",
                text: `stream frame ${frame}`,
                content: `stream frame ${frame}`,
                timestamp: "2026-01-01T00:00:02.000Z",
              },
            },
          ],
          // Model a decoded full wire frame: stable history projections have
          // equal values but fresh object identities on every commit.
          renderState: structuredClone({
            ...renderState,
            based_on_seq: seq,
            rows: nextRows,
          }),
        });
        await nextPaint();
      }
      const activeSnapshot = snapshot();
      const tailRowRenders = activeSnapshot.rowKeys[tailKey] || 0;
      const stableRowRenders = Object.entries(activeSnapshot.rowKeys)
        .filter(([key]) => key !== tailKey)
        .reduce((sum, [, value]) => sum + value, 0);

      delete window.__garyxTranscriptRenderProbe;
      return {
        frames: frameCount,
        rows: rowCount,
        toolRows,
        background: {
          appShellRenders: background.appShellRenders,
          threadPageRenders: background.threadPageRenders,
          rowRenders: background.rowRenders,
        },
        active: {
          appShellRenders: activeSnapshot.appShellRenders,
          threadPageRenders: activeSnapshot.threadPageRenders,
          rowRenders: activeSnapshot.rowRenders,
          stableRowRenders,
          tailRowRenders,
          uniqueRowKeys: Object.keys(activeSnapshot.rowKeys).length,
        },
      };
    },
    {
      activeThreadId: ACTIVE_THREAD_ID,
      backgroundThreadId: BACKGROUND_THREAD_ID,
      frameCount: frames,
      rowCount: rows,
    },
  );
}

async function main() {
  const frames = parsePositiveInteger("--frames", DEFAULT_FRAMES);
  const rows = parsePositiveInteger("--rows", DEFAULT_ROWS);
  const expectation = parseExpectation();
  const userDataDir = await mkdtemp(path.join(os.tmpdir(), "garyx-render-oracle-"));
  const stateFile = path.join(userDataDir, "garyx-desktop-state.json");
  const port = await freePort();
  await mkdir(userDataDir, { recursive: true });
  await writeFile(
    stateFile,
    JSON.stringify({
      settings: {
        gatewayUrl: "http://127.0.0.1:31337",
        gatewayAuthToken: gatewayAuthToken(),
        gatewayHeaders: "",
        accountId: "render-oracle",
        fromId: "render-oracle",
        agentId: "claude",
        timeoutSeconds: 60,
      },
      workspaces: [],
    }),
  );

  const child = spawn(
    "npm",
    ["run", "dev", "--", "--remoteDebuggingPort", String(port)],
    {
      cwd: projectDir,
      detached: true,
      env: {
        ...process.env,
        GARYX_DESKTOP_USER_DATA_PATH: userDataDir,
      },
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  let browser;
  try {
    await waitForCdp(port, child);
    browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
    let page = browser
      .contexts()
      .flatMap((context) => context.pages())
      .find((candidate) => /https?:\/\/(localhost|127\.0\.0\.1):/.test(candidate.url()));
    if (!page) {
      const context = browser.contexts()[0];
      await new Promise((resolve) => setTimeout(resolve, 500));
      page = context
        ?.pages()
        .find((candidate) => /https?:\/\/(localhost|127\.0\.0\.1):/.test(candidate.url()));
    }
    if (!page) {
      throw new Error("isolated Garyx renderer page was not found");
    }
    const result = await runOracle(page, frames, rows);
    assertExpectation(result, expectation);
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  } finally {
    await browser?.close().catch(() => {});
    await stopProcessGroup(child);
    await rm(userDataDir, {
      force: true,
      maxRetries: 10,
      recursive: true,
      retryDelay: 200,
    });
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
});
