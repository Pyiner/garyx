#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process";
import { createServer } from "node:net";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const projectDir = process.env.GARYX_TAIL_REPRO_PROJECT_DIR
  ? path.resolve(process.env.GARYX_TAIL_REPRO_PROJECT_DIR)
  : path.resolve(scriptDir, "..");
const ACTIVE_THREAD_ID = "__garyx_new_thread_draft__";

function expectedMode() {
  const index = process.argv.indexOf("--expect");
  if (index < 0) {
    return null;
  }
  const value = process.argv[index + 1];
  if (value !== "fixed") {
    throw new Error("--expect only supports fixed");
  }
  return value;
}

function assertNearZero(value, label, tolerance = 0.5) {
  if (typeof value !== "number" || Math.abs(value) > tolerance) {
    throw new Error(`${label}: expected |delta| <= ${tolerance}, got ${value}`);
  }
}

function assertFixed(result) {
  if (result.structure.thinkingInContent !== true) {
    throw new Error("thinking row is not inside MessageScrollerContent");
  }
  if (!["static", "relative"].includes(result.structure.thinkingPosition)) {
    throw new Error(
      `thinking row must participate in normal flow, got ${result.structure.thinkingPosition}`,
    );
  }
  assertNearZero(result.bottomShow.delta.anchorTop, "bottom show anchor");
  assertNearZero(result.bottomShow.delta.scrollTop, "bottom show scrollTop");
  assertNearZero(result.bottomHide.delta.anchorTop, "bottom hide anchor");
  assertNearZero(result.bottomHide.delta.scrollTop, "bottom hide scrollTop");
  assertNearZero(result.nonBottomShow.delta.anchorTop, "non-bottom show anchor");
  assertNearZero(
    result.nonBottomShow.delta.scrollTop,
    "non-bottom show scrollTop",
  );
  assertNearZero(
    result.atomicFirstOutput.delta.anchorTop,
    "thinking-to-first-output anchor",
  );
  assertNearZero(
    result.atomicFirstOutput.delta.scrollTop,
    "thinking-to-first-output scrollTop",
  );
  const movementMismatch =
    result.scrollMovement.delta.thinkingTop -
    result.scrollMovement.delta.anchorTop;
  assertNearZero(movementMismatch, "thinking/content scroll movement mismatch");
  if (
    typeof result.composerClearance !== "number" ||
    result.composerClearance < 16
  ) {
    throw new Error(
      `thinking/composer clearance: expected >= 16, got ${result.composerClearance}`,
    );
  }
}

function gatewayAuthToken() {
  const result = spawnSync("garyx", ["gateway", "token", "--json"], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  if (result.error || result.status !== 0) {
    throw new Error("Could not read the local gateway token");
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

async function runProbe(page) {
  const rendererUrl = page.url().split("#", 1)[0];
  await page.goto(`${rendererUrl}#/new`);
  await page.waitForFunction(() => Boolean(window.__garyxGatewayMirror), null, {
    timeout: 30_000,
  });
  await page.waitForSelector(".thread-main.new-thread-centered", {
    timeout: 30_000,
  });

  return page.evaluate(async ({ activeThreadId }) => {
    const mirror = window.__garyxGatewayMirror;
    if (!mirror) {
      throw new Error("dev GatewayMirror handle is unavailable");
    }

    const deterministicGeometryStyle = document.createElement("style");
    deterministicGeometryStyle.textContent = `
      .messages-item {
        content-visibility: visible !important;
        contain-intrinsic-size: none !important;
      }
    `;
    document.head.append(deterministicGeometryStyle);

    const nextPaint = () =>
      new Promise((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(resolve)),
      );
    const waitFor = async (predicate, message) => {
      for (let attempt = 0; attempt < 120; attempt += 1) {
        if (predicate()) {
          return;
        }
        await nextPaint();
      }
      throw new Error(message);
    };

    const events = [];
    const rows = [];
    let seq = 0;
    const rowCount = 48;
    for (let index = 0; index < rowCount; index += 1) {
      const userSeq = ++seq;
      const userId = `tail-repro-user:${userSeq - 1}`;
      events.push({
        type: "committed_message",
        runId: "tail-repro-seed",
        threadId: activeThreadId,
        seq: userSeq,
        message: {
          id: userId,
          seq: userSeq,
          role: "user",
          text: `question ${index}: ${"context ".repeat(3)}`,
          content: `question ${index}: ${"context ".repeat(3)}`,
          timestamp: "2026-01-01T00:00:00.000Z",
        },
      });

      const activity = [];
      if (index < rowCount - 1) {
        const assistantSeq = ++seq;
        const assistantId = `tail-repro-assistant:${assistantSeq - 1}`;
        events.push({
          type: "committed_message",
          runId: "tail-repro-seed",
          threadId: activeThreadId,
          seq: assistantSeq,
          message: {
            id: assistantId,
            seq: assistantSeq,
            role: "assistant",
            text: `answer ${index}: ${"evidence ".repeat(8)}`,
            content: `answer ${index}: ${"evidence ".repeat(8)}`,
            timestamp: "2026-01-01T00:00:01.000Z",
          },
        });
        activity.push({
          kind: "assistant_reply",
          id: `tail-repro-reply:${index}`,
          message: { id: assistantId, seq: assistantSeq, role: "assistant" },
          streaming: false,
        });
      }

      rows.push({
        kind: "user_turn",
        id: `tail-repro-turn:${index}`,
        user: { id: userId, seq: userSeq, role: "user" },
        activity,
        capsule_cards: [],
        started_at: "2026-01-01T00:00:00.000Z",
        finished_at:
          index < rowCount - 1 ? "2026-01-01T00:00:01.000Z" : null,
      });
    }

    let frameSeq = seq;
    const renderState = (tailActivity, nextRows = rows, basedOnSeq = frameSeq) => ({
      based_on_seq: basedOnSeq,
      rows: structuredClone(nextRows),
      tailActivity,
      activeToolGroupId: null,
      progress_locus: tailActivity === "thinking" ? "tail" : "none",
      filtered_placeholders: [],
    });
    const ingest = (
      tailActivity,
      nextRows = rows,
      nextEvents = [],
      basedOnSeq = frameSeq,
    ) => {
      frameSeq = Math.max(frameSeq, basedOnSeq);
      return mirror.ingest({
        type: "thread_render_frame",
        threadId: activeThreadId,
        events: nextEvents,
        renderState: renderState(tailActivity, nextRows, frameSeq),
      });
    };

    mirror.ingest({
      type: "thread_render_frame",
      threadId: activeThreadId,
      events,
      renderState: renderState("none"),
    });
    await waitFor(
      () => document.querySelectorAll(".messages-item").length === rows.length,
      "seed transcript did not render",
    );

    const viewport = document.querySelector(".messages");
    const content = document.querySelector(".messages-content");
    const composer = document.querySelector(".composer-shell-wrap");
    if (!(viewport instanceof HTMLElement) || !(content instanceof HTMLElement)) {
      throw new Error("message viewport/content missing");
    }
    if (!(composer instanceof HTMLElement)) {
      throw new Error("composer missing");
    }

    const thinkingLabel = () =>
      document.querySelector(".message-loading-label--thinking");
    const stableAnchor = () => {
      const items = Array.from(content.querySelectorAll(".messages-item"));
      return items.at(-2) || items.at(-1) || null;
    };
    const firstVisibleItem = () => {
      const viewportRect = viewport.getBoundingClientRect();
      return Array.from(content.querySelectorAll(".messages-item")).find((item) => {
        const rect = item.getBoundingClientRect();
        return rect.bottom > viewportRect.top + 1 && rect.top < viewportRect.bottom - 1;
      }) || null;
    };
    const rect = (element) => {
      if (!(element instanceof Element)) {
        return null;
      }
      const value = element.getBoundingClientRect();
      return {
        top: value.top,
        bottom: value.bottom,
        height: value.height,
      };
    };
    const snapshot = () => {
      const label = thinkingLabel();
      const anchor = stableAnchor();
      const firstVisible = firstVisibleItem();
      const labelRect = rect(label);
      const composerRect = rect(composer);
      return {
        scrollTop: viewport.scrollTop,
        scrollHeight: viewport.scrollHeight,
        clientHeight: viewport.clientHeight,
        distanceToBottom:
          viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight,
        anchorTop: rect(anchor)?.top ?? null,
        firstVisibleId:
          firstVisible instanceof HTMLElement ? firstVisible.dataset.messageId ?? null : null,
        firstVisibleTop: rect(firstVisible)?.top ?? null,
        thinkingTop: labelRect?.top ?? null,
        thinkingBottom: labelRect?.bottom ?? null,
        thinkingInContent: label ? content.contains(label) : null,
        thinkingPosition: label
          ? getComputedStyle(
              label.closest(
                '[data-tail-thinking-row="true"], .messages-tail-thinking',
              ) || label,
            ).position
          : null,
        composerTop: composerRect?.top ?? null,
        composerClearance:
          labelRect && composerRect ? composerRect.top - labelRect.bottom : null,
      };
    };
    const delta = (before, after) => ({
      scrollTop: after.scrollTop - before.scrollTop,
      scrollHeight: after.scrollHeight - before.scrollHeight,
      anchorTop:
        before.anchorTop === null || after.anchorTop === null
          ? null
          : after.anchorTop - before.anchorTop,
      firstVisibleTop:
        before.firstVisibleId !== after.firstVisibleId ||
        before.firstVisibleTop === null ||
        after.firstVisibleTop === null
          ? null
          : after.firstVisibleTop - before.firstVisibleTop,
      thinkingTop:
        before.thinkingTop === null || after.thinkingTop === null
          ? null
          : after.thinkingTop - before.thinkingTop,
    });
    const scrollToBottom = async () => {
      viewport.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: 1 }));
      viewport.scrollTop = viewport.scrollHeight;
      viewport.dispatchEvent(new Event("scroll", { bubbles: true }));
      await nextPaint();
    };
    const scrollByUser = async (amount) => {
      viewport.dispatchEvent(
        new WheelEvent("wheel", { bubbles: true, deltaY: amount }),
      );
      viewport.scrollTop += amount;
      viewport.dispatchEvent(new Event("scroll", { bubbles: true }));
      await nextPaint();
    };

    await scrollToBottom();
    const bottomBeforeShow = snapshot();
    ingest("thinking");
    await waitFor(() => Boolean(thinkingLabel()), "thinking did not appear");
    const bottomAfterShow = snapshot();

    ingest("none");
    await waitFor(() => !thinkingLabel(), "thinking did not disappear");
    const bottomAfterHide = snapshot();

    ingest("thinking");
    await waitFor(() => Boolean(thinkingLabel()), "thinking did not reappear");
    await scrollToBottom();
    const scrollMovementBefore = snapshot();
    await scrollByUser(-120);
    const scrollMovementAfter = snapshot();

    ingest("none");
    await waitFor(() => !thinkingLabel(), "thinking did not clear for non-bottom case");
    await scrollToBottom();
    await scrollByUser(-320);
    const nonBottomBeforeShow = snapshot();
    ingest("thinking");
    await waitFor(() => Boolean(thinkingLabel()), "non-bottom thinking did not appear");
    const nonBottomAfterShow = snapshot();

    await scrollToBottom();
    const atomicBefore = snapshot();
    const assistantSeq = seq + 1;
    const assistantId = `tail-repro-assistant:${assistantSeq - 1}`;
    const nextRows = structuredClone(rows);
    nextRows.at(-1).activity = [
      {
        kind: "assistant_reply",
        id: "tail-repro-first-reply",
        message: { id: assistantId, seq: assistantSeq, role: "assistant" },
        streaming: true,
      },
    ];
    const assistantEvent = {
      type: "committed_message",
      runId: "tail-repro-active",
      threadId: activeThreadId,
      seq: assistantSeq,
      message: {
        id: assistantId,
        seq: assistantSeq,
        role: "assistant",
        text: "tail-first-output-marker",
        content: "tail-first-output-marker",
        timestamp: "2026-01-01T00:00:02.000Z",
      },
    };
    const atomicIngestResult = ingest(
      "none",
      nextRows,
      [assistantEvent],
      assistantSeq,
    );
    let atomicRendered = false;
    for (let attempt = 0; attempt < 120; attempt += 1) {
      atomicRendered =
        !thinkingLabel() &&
        content.textContent.includes("tail-first-output-marker");
      if (atomicRendered) {
        break;
      }
      await nextPaint();
    }
    if (!atomicRendered) {
      const threadSnapshot = mirror.getThreadSnapshot(activeThreadId);
      throw new Error(
        `thinking did not atomically transition to first output: ${JSON.stringify({
          atomicIngestResult,
          messageTail: threadSnapshot.messages.slice(-3).map((message) => ({
            id: message.id,
            seq: message.seq,
            role: message.role,
            text: message.text,
          })),
          renderBasedOnSeq: threadSnapshot.renderState?.based_on_seq,
          renderTailActivity: threadSnapshot.renderState?.tailActivity,
          renderTailRow: threadSnapshot.renderState?.rows.at(-1),
          contentTextTail: content.textContent.slice(-200),
        })}`,
      );
    }
    const atomicAfter = snapshot();

    return {
      structure: {
        thinkingInContent: bottomAfterShow.thinkingInContent,
        thinkingPosition: bottomAfterShow.thinkingPosition,
      },
      bottomShow: {
        before: bottomBeforeShow,
        after: bottomAfterShow,
        delta: delta(bottomBeforeShow, bottomAfterShow),
      },
      bottomHide: {
        before: bottomAfterShow,
        after: bottomAfterHide,
        delta: delta(bottomAfterShow, bottomAfterHide),
      },
      nonBottomShow: {
        before: nonBottomBeforeShow,
        after: nonBottomAfterShow,
        delta: delta(nonBottomBeforeShow, nonBottomAfterShow),
      },
      scrollMovement: {
        before: scrollMovementBefore,
        after: scrollMovementAfter,
        delta: delta(scrollMovementBefore, scrollMovementAfter),
      },
      composerClearance: bottomAfterShow.composerClearance,
      atomicFirstOutput: {
        before: atomicBefore,
        after: atomicAfter,
        delta: delta(atomicBefore, atomicAfter),
      },
    };
  }, { activeThreadId: ACTIVE_THREAD_ID });
}

async function main() {
  const expectation = expectedMode();
  const userDataDir = await mkdtemp(path.join(os.tmpdir(), "garyx-tail-thinking-"));
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
        accountId: "tail-thinking-repro",
        fromId: "tail-thinking-repro",
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
      await new Promise((resolve) => setTimeout(resolve, 500));
      page = browser
        .contexts()
        .flatMap((context) => context.pages())
        .find((candidate) => /https?:\/\/(localhost|127\.0\.0\.1):/.test(candidate.url()));
    }
    if (!page) {
      throw new Error("isolated Garyx renderer page was not found");
    }
    const result = await runProbe(page);
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
    if (expectation === "fixed") {
      assertFixed(result);
    }
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
  console.error(error instanceof Error ? error.stack || error.message : String(error));
  process.exitCode = 1;
});
