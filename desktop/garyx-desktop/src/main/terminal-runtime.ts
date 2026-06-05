import { randomUUID } from "node:crypto";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { existsSync } from "node:fs";
import { basename } from "node:path";
import { homedir } from "node:os";

import type {
  CreateTerminalSessionInput,
  DesktopTerminalEvent,
  DesktopTerminalSession,
  DesktopTerminalState,
  TerminalSessionInput,
  TerminalWriteInput,
} from "@shared/contracts";
import type { IpcMainEvent, IpcMainInvokeEvent, WebContents } from "electron";

const MAX_TERMINAL_OUTPUT_LENGTH = 160_000;

type TerminalRecord = DesktopTerminalSession & {
  process: ChildProcessWithoutNullStreams;
};

const ansiPattern =
  // eslint-disable-next-line no-control-regex
  /\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~]|\][^\x07]*(?:\x07|\x1B\\))/g;

function stripAnsi(value: string): string {
  return value.replace(ansiPattern, "");
}

function normalizeCwd(value?: string | null): string {
  const candidate = value?.trim();
  if (candidate && existsSync(candidate)) {
    return candidate;
  }
  return process.cwd();
}

function compactCwd(cwd: string): string {
  const home = homedir();
  if (cwd === home) {
    return "~";
  }
  if (cwd.startsWith(`${home}/`)) {
    return `~/${cwd.slice(home.length + 1)}`;
  }
  return cwd;
}

function terminalTitle(cwd: string): string {
  const user = process.env.USER || "garyx";
  const folder = basename(cwd) || compactCwd(cwd);
  return `${user}: ${folder}`;
}

function appendOutput(record: TerminalRecord, chunk: string): void {
  const nextOutput = `${record.output}${chunk}`;
  record.output =
    nextOutput.length > MAX_TERMINAL_OUTPUT_LENGTH
      ? nextOutput.slice(nextOutput.length - MAX_TERMINAL_OUTPUT_LENGTH)
      : nextOutput;
  record.updatedAt = new Date().toISOString();
}

class TerminalRuntime {
  private readonly sessions = new Map<string, TerminalRecord>();

  private readonly subscribers = new Set<WebContents>();

  private activeSessionId: string | null = null;

  subscribe(event: IpcMainEvent): DesktopTerminalState {
    this.subscribers.add(event.sender);
    event.sender.once("destroyed", () => {
      this.subscribers.delete(event.sender);
    });
    return this.snapshot();
  }

  unsubscribe(event: IpcMainEvent): void {
    this.subscribers.delete(event.sender);
  }

  listState(): DesktopTerminalState {
    return this.snapshot();
  }

  createSession(input?: CreateTerminalSessionInput): DesktopTerminalState {
    const cwd = normalizeCwd(input?.cwd);
    const now = new Date().toISOString();
    const shell = process.env.SHELL || "/bin/zsh";
    const child = spawn(shell, ["-il"], {
      cwd,
      env: {
        ...process.env,
        COLORTERM: process.env.COLORTERM || "truecolor",
        FORCE_COLOR: process.env.FORCE_COLOR || "1",
        TERM: process.env.TERM || "xterm-256color",
      },
      stdio: "pipe",
    });
    child.stdin.setDefaultEncoding("utf8");

    const record: TerminalRecord = {
      id: `terminal-${randomUUID()}`,
      title: input?.title?.trim() || terminalTitle(cwd),
      cwd,
      output: "",
      running: true,
      createdAt: now,
      updatedAt: now,
      exitCode: null,
      exitSignal: null,
      process: child,
    };
    this.sessions.set(record.id, record);
    this.activeSessionId = record.id;

    const handleData = (data: Buffer) => {
      const chunk = stripAnsi(data.toString("utf8"));
      appendOutput(record, chunk);
      this.emit({
        type: "output",
        sessionId: record.id,
        data: chunk,
      });
    };

    child.stdout.on("data", handleData);
    child.stderr.on("data", handleData);
    child.on("exit", (code, signal) => {
      record.running = false;
      record.exitCode = code;
      record.exitSignal = signal;
      appendOutput(
        record,
        `\n[process exited${code == null ? "" : ` with code ${code}`}${
          signal ? ` (${signal})` : ""
        }]\n`,
      );
      this.emit({ type: "state", state: this.snapshot() });
    });

    this.emit({ type: "state", state: this.snapshot() });
    return this.snapshot();
  }

  activateSession(input: TerminalSessionInput): DesktopTerminalState {
    if (!this.sessions.has(input.sessionId)) {
      throw new Error(`terminal session not found: ${input.sessionId}`);
    }
    this.activeSessionId = input.sessionId;
    this.emit({ type: "state", state: this.snapshot() });
    return this.snapshot();
  }

  closeSession(input: TerminalSessionInput): DesktopTerminalState {
    const record = this.sessions.get(input.sessionId);
    if (!record) {
      throw new Error(`terminal session not found: ${input.sessionId}`);
    }
    record.process.kill();
    this.sessions.delete(input.sessionId);
    if (this.activeSessionId === input.sessionId) {
      this.activeSessionId = this.sessions.values().next().value?.id ?? null;
    }
    this.emit({ type: "state", state: this.snapshot() });
    return this.snapshot();
  }

  write(input: TerminalWriteInput): void {
    const record = this.sessions.get(input.sessionId);
    if (!record) {
      throw new Error(`terminal session not found: ${input.sessionId}`);
    }
    if (!record.running) {
      throw new Error("terminal session is not running");
    }
    record.process.stdin.write(input.data);
  }

  private snapshot(): DesktopTerminalState {
    return {
      activeSessionId: this.activeSessionId,
      sessions: Array.from(this.sessions.values()).map((record) => ({
        id: record.id,
        title: record.title,
        cwd: record.cwd,
        output: record.output,
        running: record.running,
        createdAt: record.createdAt,
        updatedAt: record.updatedAt,
        exitCode: record.exitCode,
        exitSignal: record.exitSignal,
      })),
    };
  }

  private emit(event: DesktopTerminalEvent): void {
    for (const subscriber of Array.from(this.subscribers)) {
      if (subscriber.isDestroyed()) {
        this.subscribers.delete(subscriber);
        continue;
      }
      subscriber.send("garyx:terminal-event", event);
    }
  }
}

const terminalRuntime = new TerminalRuntime();

export function subscribeTerminalState(event: IpcMainEvent): DesktopTerminalState {
  return terminalRuntime.subscribe(event);
}

export function unsubscribeTerminalState(event: IpcMainEvent): void {
  terminalRuntime.unsubscribe(event);
}

export function listTerminalState(): DesktopTerminalState {
  return terminalRuntime.listState();
}

export function createTerminalSession(
  _event: IpcMainInvokeEvent,
  input?: CreateTerminalSessionInput,
): DesktopTerminalState {
  return terminalRuntime.createSession(input);
}

export function activateTerminalSession(
  _event: IpcMainInvokeEvent,
  input: TerminalSessionInput,
): DesktopTerminalState {
  return terminalRuntime.activateSession(input);
}

export function closeTerminalSession(
  _event: IpcMainInvokeEvent,
  input: TerminalSessionInput,
): DesktopTerminalState {
  return terminalRuntime.closeSession(input);
}

export function writeTerminalInput(
  _event: IpcMainInvokeEvent,
  input: TerminalWriteInput,
): void {
  terminalRuntime.write(input);
}
