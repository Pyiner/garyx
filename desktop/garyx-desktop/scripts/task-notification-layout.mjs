#!/usr/bin/env node

import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";
import { createServer } from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { chromium } from "playwright";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const projectDir = path.resolve(scriptDir, "..");

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

function near(left, right, epsilon = 1) {
  return Math.abs(left - right) <= epsilon;
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
    throw new Error("could not reserve a Storybook port");
  }
  return port;
}

async function waitForServer(url, child) {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`Storybook server exited early (${child.exitCode})`);
    }
    try {
      const response = await fetch(url);
      if (response.ok) {
        return;
      }
    } catch {
      // Vite is still starting.
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error("Storybook server did not become ready");
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
  await Promise.race([
    new Promise((resolve) => child.once("exit", resolve)),
    new Promise((resolve) => setTimeout(resolve, 3_000)),
  ]);
}

async function selectStep(page, index) {
  await page.locator(`[data-step-index="${index}"]`).click();
  await page.locator(".task-notification-card").waitFor();
}

async function waitForOverflow(page, expected) {
  try {
    await page.waitForFunction(
      (value) =>
        document
          .querySelector(".task-notification-card")
          ?.getAttribute("data-overflow") === value,
      expected ? "true" : "false",
      { timeout: 5_000 },
    );
  } catch (error) {
    const measurement = await measuredBody(page);
    throw new Error(
      `expected overflow=${expected}, measured ${JSON.stringify(measurement)}`,
      { cause: error },
    );
  }
}

async function taskGeometry(page) {
  return page.evaluate(() => {
    const ordinary = document.querySelector(
      ".message-bubble.user:not(.task-notification-message)",
    );
    const task = document.querySelector(
      ".message-bubble.task-notification-message",
    );
    const content = document.querySelector(".messages-content");
    if (!(ordinary instanceof HTMLElement) || !(task instanceof HTMLElement)) {
      throw new Error("task layout story did not render both user-role rows");
    }
    const ordinaryRect = ordinary.getBoundingClientRect();
    const taskRect = task.getBoundingClientRect();
    const contentRect = content?.getBoundingClientRect();
    return {
      ordinary: {
        left: ordinaryRect.left,
        right: ordinaryRect.right,
        width: ordinaryRect.width,
      },
      task: {
        left: taskRect.left,
        right: taskRect.right,
        width: taskRect.width,
      },
      contentWidth: contentRect?.width ?? 0,
    };
  });
}

async function measuredBody(page) {
  return page.evaluate(() => {
    const card = document.querySelector(".task-notification-card");
    const viewport = document.querySelector(
      ".task-notification-body-viewport",
    );
    const content = document.querySelector(
      ".task-notification-body-content",
    );
    const rich = content?.querySelector(".message-rich");
    if (
      !(card instanceof HTMLElement) ||
      !(viewport instanceof HTMLElement) ||
      !(content instanceof HTMLElement) ||
      !(rich instanceof HTMLElement)
    ) {
      throw new Error("task notification measurement DOM is incomplete");
    }
    const style = getComputedStyle(rich);
    const lineHeight = Number.parseFloat(style.lineHeight);
    const clampHeight = Number.parseFloat(
      getComputedStyle(viewport).getPropertyValue(
        "--task-notification-clamp-height",
      ),
    );
    return {
      clampHeight,
      lineHeight,
      naturalHeight: Math.max(
        content.scrollHeight,
        content.getBoundingClientRect().height,
      ),
      overflow: card.dataset.overflow === "true",
      viewportHeight: viewport.getBoundingClientRect().height,
    };
  });
}

async function main() {
  const port = await freePort();
  const baseUrl = `http://127.0.0.1:${port}`;
  const server = spawn(
    process.execPath,
    [
      path.join(projectDir, "node_modules/vite/bin/vite.js"),
      "--config",
      "vite.storybook.config.ts",
      "--host",
      "127.0.0.1",
      "--port",
      String(port),
      "--strictPort",
    ],
    {
      cwd: projectDir,
      detached: true,
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  let browser;
  const evidence = { width: [], clamp: {} };

  try {
    await waitForServer(`${baseUrl}/storybook.html`, server);
    browser = await chromium.launch({ headless: true });
    const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
    await page.goto(`${baseUrl}/storybook.html`);
    await page.locator('[data-story-id="task-notification"]').click();
    await page.locator(".task-notification-card").waitFor();

    for (const viewport of [
      { width: 1440, height: 900 },
      { width: 900, height: 900 },
    ]) {
      await page.setViewportSize(viewport);
      await page.waitForTimeout(100);
      const geometry = await taskGeometry(page);
      assert(
        near(geometry.ordinary.right, geometry.task.right),
        `ordinary/card trailing edges diverged at ${viewport.width}px: ${JSON.stringify(geometry)}`,
      );
      assert(
        near(geometry.ordinary.width, geometry.task.width),
        `ordinary/card shared cap diverged at ${viewport.width}px: ${JSON.stringify(geometry)}`,
      );
      assert(
        geometry.task.width <= geometry.contentWidth * 0.77 + 1,
        `task card exceeded the shared 77% owner at ${viewport.width}px`,
      );
      evidence.width.push({ viewport: viewport.width, ...geometry });
    }

    await selectStep(page, 0);
    await waitForOverflow(page, false);
    assert(
      (await page.locator(".task-notification-expand").count()) === 0,
      "short body exposed an expand affordance",
    );
    evidence.clamp.short = await measuredBody(page);

    await page.setViewportSize({ width: 900, height: 900 });
    await selectStep(page, 1);
    await waitForOverflow(page, true);
    evidence.clamp.wrapNarrow = await measuredBody(page);
    await page.setViewportSize({ width: 1800, height: 900 });
    await waitForOverflow(page, false);
    evidence.clamp.wrapWide = await measuredBody(page);

    const beforeFontScale = await measuredBody(page);
    await page.addStyleTag({
      content:
        ".task-notification-body .message-rich, .task-notification-body .message-rich * { font-size: 24px !important; line-height: 1.8 !important; }",
    });
    try {
      await page.waitForFunction(
        (before) => {
          const viewport = document.querySelector(
            ".task-notification-body-viewport",
          );
          if (!(viewport instanceof HTMLElement)) return false;
          const after = Number.parseFloat(
            getComputedStyle(viewport).getPropertyValue(
              "--task-notification-clamp-height",
            ),
          );
          return Number.isFinite(after) && Math.abs(after - before) > 1;
        },
        beforeFontScale.clampHeight,
        { timeout: 5_000 },
      );
    } catch (error) {
      throw new Error(
        `font-scale clamp did not remeasure: before=${JSON.stringify(beforeFontScale)} after=${JSON.stringify(await measuredBody(page))}`,
        { cause: error },
      );
    }
    evidence.clamp.fontScale = await measuredBody(page);
    assert(
      near(
        evidence.clamp.fontScale.clampHeight,
        evidence.clamp.fontScale.lineHeight * 10,
        0.75,
      ),
      "font-scale remeasure did not preserve the ten-line-box clamp",
    );

    await page.setViewportSize({ width: 1200, height: 900 });
    await selectStep(page, 2);
    await waitForOverflow(page, true);
    evidence.clamp.explicitLines = await measuredBody(page);

    await selectStep(page, 3);
    await waitForOverflow(page, true);
    assert((await page.locator(".task-notification-card pre").count()) > 0, "code missing");
    assert((await page.locator(".task-notification-card table").count()) > 0, "table missing");
    const link = page.locator(".task-notification-card a").first();
    assert((await link.count()) === 1, "markdown link missing");
    const popupPromise = page.waitForEvent("popup");
    await link.click();
    const popup = await popupPromise;
    await popup.close();
    assert(
      (await page.locator('[role="dialog"]').count()) === 0,
      "interactive descendant click opened the task dialog",
    );

    await page.evaluate(() => {
      const paragraph = document.querySelector(
        ".task-notification-body-content li",
      );
      if (!paragraph?.firstChild) {
        throw new Error("selectable task body text is missing");
      }
      const range = document.createRange();
      range.selectNodeContents(paragraph);
      const selection = window.getSelection();
      selection?.removeAllRanges();
      selection?.addRange(range);
      paragraph.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    assert(
      (await page.locator('[role="dialog"]').count()) === 0,
      "selecting task body text opened the task dialog",
    );
    await page.evaluate(() => window.getSelection()?.removeAllRanges());

    await page.locator(".task-notification-body-content li").first().click();
    const dialog = page.locator('[role="dialog"]');
    await dialog.waitFor();
    assert(
      (await dialog.getByText("await verify(manifest)", { exact: false }).count()) > 0,
      "expanded dialog did not retain the complete body",
    );
    for (let index = 0; index < 8; index += 1) {
      await page.keyboard.press("Tab");
      assert(
        await page.evaluate(() =>
          Boolean(document.activeElement?.closest('[role="dialog"]')),
        ),
        "Radix focus escaped the open task dialog",
      );
    }
    await page.keyboard.press("Escape");
    await dialog.waitFor({ state: "detached" });
    await page.waitForFunction(
      () => document.activeElement?.classList.contains("task-notification-card"),
      null,
      { timeout: 2_000 },
    );

    await page.locator(".task-notification-title").click();
    await dialog.waitFor();
    await page.evaluate(() => {
      const button = document.querySelector('[data-step-index="0"]');
      if (!(button instanceof HTMLButtonElement)) {
        throw new Error("snapshot eviction step is unavailable");
      }
      button.click();
    });
    await page.getByText("All focused tests pass.", { exact: true }).waitFor();
    assert(
      (await dialog.getByText("await verify(manifest)", { exact: false }).count()) > 0,
      "dialog snapshot disappeared when its render row was evicted",
    );
    await page.keyboard.press("Escape");
    await dialog.waitFor({ state: "detached" });

    let imageRequestStarted = false;
    await page.route(
      "https://example.test/task-notification-late-image.svg",
      async (route) => {
        imageRequestStarted = true;
        await new Promise((resolve) => setTimeout(resolve, 600));
        await route.fulfill({
          body: '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="900"><rect width="800" height="900" fill="#d8dee9"/></svg>',
          contentType: "image/svg+xml",
          status: 200,
        });
      },
    );
    await selectStep(page, 4);
    await waitForOverflow(page, false);
    assert(imageRequestStarted, "late intrinsic image request did not start");
    await page.waitForFunction(() => {
      const image = document.querySelector(
        '.task-notification-card img[alt="Late intrinsic task result"]',
      );
      return image instanceof HTMLImageElement && image.complete && image.naturalHeight > 0;
    });
    await waitForOverflow(page, true);
    evidence.clamp.lateImage = await measuredBody(page);

    const fontBytes = await readFile(
      path.join(
        projectDir,
        "node_modules/katex/dist/fonts/KaTeX_Typewriter-Regular.woff2",
      ),
    );
    await page.route(`${baseUrl}/task-notification-late-font.woff2`, async (route) => {
      await new Promise((resolve) => setTimeout(resolve, 300));
      await route.fulfill({
        body: fontBytes,
        contentType: "font/woff2",
        status: 200,
      });
    });
    await page.setViewportSize({ width: 1200, height: 900 });
    await selectStep(page, 1);
    const beforeLateFont = await measuredBody(page);
    await page.evaluate(async (fontUrl) => {
      const face = new FontFace("TaskNotificationLateFont", `url(${fontUrl})`);
      document.fonts.add(face);
      const style = document.createElement("style");
      style.textContent =
        '.task-notification-body .message-rich, .task-notification-body .message-rich * { font-family: "TaskNotificationLateFont", monospace !important; }';
      document.head.append(style);
      await face.load();
    }, `${baseUrl}/task-notification-late-font.woff2`);
    await page.waitForFunction(() =>
      document.fonts.check('16px "TaskNotificationLateFont"'),
    );
    await page.waitForTimeout(100);
    const afterLateFont = await measuredBody(page);
    assert(
      Math.abs(afterLateFont.naturalHeight - beforeLateFont.naturalHeight) > 0.5,
      `late font did not settle intrinsic layout: before=${JSON.stringify(beforeLateFont)} after=${JSON.stringify(afterLateFont)}`,
    );
    evidence.clamp.lateFont = { before: beforeLateFont, after: afterLateFont };

    console.log(JSON.stringify(evidence, null, 2));
    console.log("task notification real-layout checks passed");
  } finally {
    await browser?.close();
    await stopProcessGroup(server);
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
