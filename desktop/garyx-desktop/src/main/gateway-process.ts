import { execFile as execFileCallback } from 'node:child_process';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { promisify } from 'node:util';

const execFile = promisify(execFileCallback);

const DEFAULT_GATEWAY_PORT = Number(process.env.GARYBOT_PORT || '31337');
const LAUNCHD_SERVICE_NAME = 'com.garyx.agent';
const LAUNCHD_PLIST_PATH = join(homedir(), 'Library/LaunchAgents', `${LAUNCHD_SERVICE_NAME}.plist`);
const LAUNCHCTL_BIN = '/bin/launchctl';
const LSOF_BIN = '/usr/sbin/lsof';

export type GatewayStatus = 'starting' | 'running' | 'stopped' | 'error';

let currentStatus: GatewayStatus = 'stopped';
let statusChangeHandler: (() => void) | null = null;
let startupPromise: Promise<void> | null = null;

function setStatus(nextStatus: GatewayStatus): void {
  if (currentStatus === nextStatus) {
    return;
  }
  currentStatus = nextStatus;
  statusChangeHandler?.();
}

function uid(): number {
  const value = process.getuid?.();
  if (typeof value !== 'number') {
    throw new Error('launchd control requires a POSIX uid');
  }
  return value;
}

// Domains the gateway can be installed into, in the desktop's preferred order.
// The CLI may bootstrap into `gui/<uid>` (Aqua sessions) or `user/<uid>`
// (SSH/headless or the Aqua fallback), so the desktop must probe both rather
// than assuming the GUI domain.
function candidateDomains(): string[] {
  const id = uid();
  return [`gui/${id}`, `user/${id}`];
}

async function runCommand(file: string, args: string[]): Promise<string> {
  const { stdout } = await execFile(file, args, {
    encoding: 'utf8',
    env: process.env,
  });
  return stdout.trim();
}

async function tryRunCommand(file: string, args: string[]): Promise<string | null> {
  try {
    return await runCommand(file, args);
  } catch {
    return null;
  }
}

async function loadedDomainOutput(): Promise<{ target: string; output: string } | null> {
  for (const domain of candidateDomains()) {
    const target = `${domain}/${LAUNCHD_SERVICE_NAME}`;
    const output = await tryRunCommand(LAUNCHCTL_BIN, ['print', target]);
    if (output !== null) {
      return { target, output };
    }
  }
  return null;
}

async function getLaunchdPid(): Promise<number | null> {
  const loaded = await loadedDomainOutput();
  if (!loaded) {
    return null;
  }
  const match = loaded.output.match(/\bpid = (\d+)\b/);
  if (!match) {
    return null;
  }
  const pid = Number(match[1]);
  return Number.isFinite(pid) && pid > 0 ? pid : null;
}

async function ensureLaunchdLoaded(): Promise<void> {
  if (await loadedDomainOutput()) {
    return;
  }
  // Not loaded in any domain yet. The desktop launcher runs inside the Aqua
  // session, so bootstrap into the GUI domain to match historical installs.
  try {
    await runCommand(LAUNCHCTL_BIN, ['bootstrap', `gui/${uid()}`, LAUNCHD_PLIST_PATH]);
  } catch (error) {
    const stderr = error instanceof Error ? error.message : String(error);
    if (!stderr.includes('service already loaded')) {
      throw error;
    }
  }
}

async function kickstartLaunchd(): Promise<void> {
  const loaded = await loadedDomainOutput();
  if (!loaded) {
    throw new Error('launchd gateway is not loaded in any domain');
  }
  await runCommand(LAUNCHCTL_BIN, ['kickstart', '-k', loaded.target]);
}

async function getListeningPid(port: number): Promise<number | null> {
  const output = await tryRunCommand(LSOF_BIN, ['-nP', `-iTCP:${port}`, '-sTCP:LISTEN', '-t']);
  if (!output) {
    return null;
  }
  const pidText = output.split(/\s+/).find((entry) => entry.length > 0) || '';
  const pid = Number(pidText);
  return Number.isFinite(pid) && pid > 0 ? pid : null;
}

async function waitForProcessExit(pid: number, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      process.kill(pid, 0);
      await delay(200);
    } catch {
      return;
    }
  }
  throw new Error(`process ${pid} did not exit in time`);
}

async function terminateProcess(pid: number): Promise<void> {
  try {
    process.kill(pid, 'SIGTERM');
  } catch {
    return;
  }
  try {
    await waitForProcessExit(pid, 5_000);
    return;
  } catch {
    process.kill(pid, 'SIGKILL');
    await waitForProcessExit(pid, 2_000);
  }
}

async function waitForListeningPid(port: number, timeoutMs: number): Promise<number | null> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const pid = await getListeningPid(port);
    if (pid) {
      return pid;
    }
    await delay(300);
  }
  return null;
}

async function ensureLaunchdGateway(port: number): Promise<void> {
  setStatus('starting');

  const launchdPid = await getLaunchdPid();
  const existingListenerPid = await getListeningPid(port);
  if (existingListenerPid && launchdPid && existingListenerPid === launchdPid) {
    setStatus('running');
    return;
  }

  if (existingListenerPid && existingListenerPid !== launchdPid) {
    await terminateProcess(existingListenerPid);
  }

  await ensureLaunchdLoaded();
  await kickstartLaunchd();

  const listenerPid = await waitForListeningPid(port, 30_000);
  if (!listenerPid) {
    throw new Error(`launchd gateway did not start listening on port ${port}`);
  }

  const refreshedLaunchdPid = await getLaunchdPid();
  if (refreshedLaunchdPid && listenerPid !== refreshedLaunchdPid) {
    const launchdListenerPid = await waitForListeningPid(port, 5_000);
    if (launchdListenerPid !== refreshedLaunchdPid) {
      throw new Error(
        `port ${port} is owned by pid ${listenerPid}, but launchd reports pid ${refreshedLaunchdPid}`,
      );
    }
  }

  setStatus('running');
}

export function getGatewayStatus(): GatewayStatus {
  return currentStatus;
}

export function setOnStatusChange(handler: (() => void) | null): void {
  statusChangeHandler = handler;
}

export function startGateway(port = DEFAULT_GATEWAY_PORT, _host = '0.0.0.0'): void {
  if (startupPromise) {
    return;
  }
  startupPromise = ensureLaunchdGateway(port)
    .catch((error) => {
      console.error('failed to ensure launchd gateway', error);
      setStatus('error');
    })
    .finally(() => {
      startupPromise = null;
    });
}

export function stopGateway(): void {
  if (startupPromise) {
    void startupPromise.catch(() => {});
  }
}
