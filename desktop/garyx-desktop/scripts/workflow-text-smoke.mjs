import assert from 'node:assert/strict';
import { execFile as execFileCallback } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';

import { _electron as electron } from 'playwright';
import { buildDesktopElectronLaunchEnv } from './electron-launch-env.mjs';

const execFile = promisify(execFileCallback);
const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const projectDir = path.resolve(scriptDir, '..');
const repoRoot = path.resolve(projectDir, '../..');
const fallbackCli = path.join(repoRoot, 'target/release/garyx');
const garyxCli = process.env.GARYX_CLI || (existsSync(fallbackCli) ? fallbackCli : 'garyx');
const garyxConfig = process.env.GARYX_CONFIG || '';
const gatewayUrl = process.env.GARYX_GATEWAY_URL || 'http://127.0.0.1:31337';
const runId = Date.now().toString(36);
const workflowId = `mac-ui-text-smoke-${runId}`;
const workflowName = `Mac UI Text Smoke ${runId}`;
const title = `Mac UI workflow text input ${runId}`;
const inputText = 'Use plain text input from the Mac workflow task dialog.';
const artifactDir = path.join(projectDir, 'test-artifacts');
const artifactPath = path.join(artifactDir, 'workflow-text-smoke.png');
const artifactCreatePanelPath = path.join(artifactDir, 'workflow-task-create-panel.png');
const artifactTaskCardPath = path.join(artifactDir, 'workflow-task-card.png');
const artifactRunsPanelPath = path.join(artifactDir, 'workflow-runs-panel.png');

async function runGaryx(args) {
  const cliArgs = garyxConfig ? ['--config', garyxConfig, ...args] : args;
  const { stdout } = await execFile(garyxCli, cliArgs, {
    cwd: repoRoot,
    maxBuffer: 1024 * 1024 * 4,
  });
  return stdout;
}

async function gatewayJson(pathname) {
  const response = await fetch(new URL(pathname, gatewayUrl));
  if (!response.ok) {
    throw new Error(`Gateway ${pathname} failed: ${response.status} ${await response.text()}`);
  }
  return response.json();
}

async function waitForWorkflowResult() {
  const deadline = Date.now() + 60_000;
  while (Date.now() < deadline) {
    const list = await gatewayJson('/api/workflows?limit=20');
    const run = list.workflows?.find(
      (candidate) =>
        candidate.workflowDefinitionId === workflowId &&
        candidate.name === workflowName &&
        candidate.status === 'succeeded',
    );
    if (run?.workflowRunId) {
      const detail = await gatewayJson(`/api/workflows/${encodeURIComponent(run.workflowRunId)}`);
      if (detail.workflow?.result?.input === inputText) {
        return detail.workflow;
      }
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new Error('Timed out waiting for workflow text smoke result');
}

async function createWorkflowPackage(rootDir, workspaceDir) {
  const packageDir = path.join(rootDir, 'mac-ui-text-smoke');
  await mkdir(packageDir, { recursive: true });
  await writeFile(
    path.join(packageDir, 'garyx.workflow.json'),
    `${JSON.stringify(
      {
        workflowId,
        version: 1,
        name: workflowName,
        description: 'Validates that workflow tasks accept plain text input from the Mac app.',
        input: { placeholder: 'Describe the workflow request.' },
        defaults: { workspaceDir },
      },
      null,
      2,
    )}\n`,
    'utf8',
  );
  await writeFile(
    path.join(packageDir, 'workflow.ts'),
    `import { workflow } from "@garyx/workflow";

await workflow({
  name: ${JSON.stringify(workflowName)},
  description: "Echo plain text workflow input for UI validation.",
  async run(ctx) {
    await ctx.log("received workflow input", {
      inputType: typeof ctx.input,
      input: ctx.input,
    });
    return {
      status: "ok",
      inputType: typeof ctx.input,
      input: ctx.input,
      hasTextInput: typeof ctx.input === "string" && ctx.input.length > 0,
      workflowRunId: ctx.workflowRunId,
    };
  },
});
`,
    'utf8',
  );
  return packageDir;
}

async function prepareDesktopState(homeDir, workspaceDir) {
  const userDataDir = path.join(homeDir, 'user-data');
  await mkdir(userDataDir, { recursive: true });
  await writeFile(
    path.join(userDataDir, 'garyx-desktop-state.json'),
    JSON.stringify(
      {
        settings: {
          gatewayUrl,
          accountId: 'mac-ui-workflow-text-smoke',
          fromId: 'mac-ui-workflow-text-smoke',
          timeoutSeconds: 180,
          languagePreference: 'en',
        },
        workspaces: [
          {
            name: path.basename(workspaceDir),
            path: workspaceDir,
            kind: 'local',
            createdAt: new Date().toISOString(),
            updatedAt: new Date().toISOString(),
            available: true,
          },
        ],
        selectedWorkspacePath: workspaceDir,
        sessions: [],
      },
      null,
      2,
    ),
    'utf8',
  );
  return userDataDir;
}

async function runDesktopFlow(userDataDir, homeDir) {
  const electronApp = await electron.launch({
    args: [projectDir],
    cwd: projectDir,
    env: buildDesktopElectronLaunchEnv({
      HOME: homeDir,
      GARYX_DESKTOP_USER_DATA_PATH: userDataDir,
    }),
  });

  try {
    const page = await electronApp.firstWindow();
    page.on('pageerror', (error) => {
      console.error('PAGE_ERROR', error.stack || error.message || String(error));
    });
    await page.waitForLoadState('domcontentloaded');
    await page.setViewportSize({ width: 1480, height: 940 });
    await page.locator('.app-shell').waitFor({ timeout: 30_000 });

    await page.getByRole('button', { name: /^Tasks$/i }).click({ timeout: 15_000 });
    await page.locator('.tasks-page').waitFor({ timeout: 20_000 });
    await page.getByRole('button', { name: /^New task$/i }).click({ timeout: 15_000 });
    await page.locator('.tasks-create-panel').waitFor({ timeout: 10_000 });

    const executorTabs = await page.locator('.tasks-executor-tabs button').allTextContents();
    assert.deepEqual(
      executorTabs.map((value) => value.replace(/\s+/g, ' ').trim()),
      ['Agent', 'Agent Team', 'Workflow'],
    );

    await page.locator('input[placeholder="Task title"]').fill(title);
    await page.locator('.tasks-executor-tabs button').nth(2).click();
    await page.waitForFunction(
      (id) =>
        [...document.querySelectorAll('.tasks-executor-panel select option')].some(
          (option) => option.value === id,
        ),
      workflowId,
      { timeout: 20_000 },
    );

    await page.locator('.tasks-executor-panel button[role="combobox"]').click();
    await page.locator('[role="option"]').filter({ hasText: workflowName }).click();

    const body = page.locator('textarea[placeholder="Describe the workflow request."]');
    await body.fill(inputText);
    assert.equal(await body.inputValue(), inputText);
    await mkdir(artifactDir, { recursive: true });
    await page.locator('.tasks-create-panel').screenshot({
      path: artifactCreatePanelPath,
    });

    await page.getByRole('combobox').filter({ hasText: /^Do not notify$/ }).waitFor({
      timeout: 10_000,
    });
    await page.getByRole('button', { name: /^Start workflow$/i }).click();
    await page.locator('.tasks-create-panel').waitFor({ state: 'detached', timeout: 30_000 });

    const card = page.locator('.tasks-card').filter({ hasText: title }).first();
    await card.waitFor({ timeout: 30_000 });
    await card.screenshot({ path: artifactTaskCardPath });
    await page.screenshot({ path: artifactPath, fullPage: true });
  } finally {
    await electronApp.close();
  }
}

async function captureWorkflowRunsPanel(userDataDir, homeDir) {
  const electronApp = await electron.launch({
    args: [projectDir],
    cwd: projectDir,
    env: buildDesktopElectronLaunchEnv({
      HOME: homeDir,
      GARYX_DESKTOP_USER_DATA_PATH: userDataDir,
    }),
  });

  try {
    const page = await electronApp.firstWindow();
    page.on('pageerror', (error) => {
      console.error('PAGE_ERROR', error.stack || error.message || String(error));
    });
    await page.waitForLoadState('domcontentloaded');
    await page.setViewportSize({ width: 1480, height: 940 });
    await page.locator('.app-shell').waitFor({ timeout: 30_000 });
    await page.getByRole('button', { name: /^Tasks$/i }).click({ timeout: 15_000 });
    await page.locator('.tasks-page').waitFor({ timeout: 20_000 });
    const card = page.locator('.tasks-card').filter({ hasText: title }).first();
    await card.waitFor({ timeout: 30_000 });
    await card.locator('.tasks-card-title').click();
    await page.waitForFunction(() => window.location.hash.startsWith('#/workflow/'), null, {
      timeout: 10_000,
    });
    const panel = page.locator('.workflow-runs-panel');
    await panel.waitFor({ timeout: 20_000 });
    await panel.locator('.workflow-run-card').first().waitFor({ timeout: 20_000 });
    await panel.screenshot({ path: artifactRunsPanelPath });
  } finally {
    await electronApp.close();
  }
}

const tempRoot = await mkdtemp(path.join(os.tmpdir(), 'garyx-workflow-text-smoke-'));
let installedPackageDir = null;
try {
  const workspaceDir = path.join(tempRoot, 'workspace');
  await mkdir(workspaceDir, { recursive: true });
  const packageDir = await createWorkflowPackage(tempRoot, workspaceDir);
  const installOutput = await runGaryx([
    'workflow',
    'definition',
    'upsert',
    '--file',
    packageDir,
    '--json',
  ]);
  installedPackageDir = JSON.parse(installOutput).workflowDefinition?.packageDir || null;
  const userDataDir = await prepareDesktopState(tempRoot, workspaceDir);
  await runDesktopFlow(userDataDir, tempRoot);
  const workflow = await waitForWorkflowResult();
  await captureWorkflowRunsPanel(userDataDir, tempRoot);
  assert.equal(workflow.result.inputType, 'string');
  assert.equal(workflow.result.input, inputText);
  assert.equal(workflow.result.hasTextInput, true);
  console.log(JSON.stringify({
    taskId: workflow.taskId,
    workflowRunId: workflow.workflowRunId,
    inputType: workflow.result.inputType,
    screenshots: {
      createPanel: artifactCreatePanelPath,
      taskCard: artifactTaskCardPath,
      workflowRunsPanel: artifactRunsPanelPath,
      fullPage: artifactPath,
    },
  }, null, 2));
} finally {
  if (installedPackageDir) {
    await rm(installedPackageDir, { recursive: true, force: true });
  }
  await rm(tempRoot, { recursive: true, force: true });
}
