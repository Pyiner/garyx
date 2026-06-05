import { execFile } from "node:child_process";
import { existsSync } from "node:fs";
import { promisify } from "node:util";

import type {
  CommitWorkspaceChangesInput,
  DesktopWorkspaceGitDetails,
  DesktopWorkspaceGitFile,
  PushWorkspaceBranchInput,
  WorkspaceGitMutationResult,
} from "@shared/contracts";
import type { IpcMainInvokeEvent } from "electron";

const execFileAsync = promisify(execFile);
const GIT_TIMEOUT_MS = 30_000;

async function runGit(
  cwd: string,
  args: string[],
): Promise<{ stdout: string; stderr: string }> {
  const { stdout, stderr } = await execFileAsync("git", args, {
    cwd,
    env: process.env,
    maxBuffer: 8 * 1024 * 1024,
    timeout: GIT_TIMEOUT_MS,
  });
  return {
    stdout: String(stdout || ""),
    stderr: String(stderr || ""),
  };
}

function parseBranchLine(line: string): {
  branch: string | null;
  ahead: number;
  behind: number;
} {
  const value = line.replace(/^##\s*/, "").trim();
  const ahead = Number.parseInt(value.match(/ahead\s+(\d+)/)?.[1] || "0", 10);
  const behind = Number.parseInt(value.match(/behind\s+(\d+)/)?.[1] || "0", 10);
  const branchPart = value
    .replace(/\s+\[.*\]$/, "")
    .split("...")[0]
    .replace(/^No commits yet on\s+/, "")
    .trim();
  const branch =
    branchPart && branchPart !== "HEAD (no branch)" ? branchPart : null;
  return { branch, ahead, behind };
}

function parseStatusLine(line: string): DesktopWorkspaceGitFile | null {
  if (line.length < 4) {
    return null;
  }
  const status = line.slice(0, 2);
  const rawPath = line.slice(3).trim();
  if (!rawPath) {
    return null;
  }
  return {
    path: rawPath,
    status,
  };
}

async function resolveRepoRoot(workspacePath: string): Promise<string | null> {
  const candidate = workspacePath.trim();
  if (!candidate || !existsSync(candidate)) {
    return null;
  }
  try {
    const { stdout } = await runGit(candidate, ["rev-parse", "--show-toplevel"]);
    return stdout.trim() || null;
  } catch {
    return null;
  }
}

export async function getWorkspaceGitDetailsForPath(
  workspacePath: string,
): Promise<DesktopWorkspaceGitDetails> {
  const workspaceDir = workspacePath.trim();
  const repoRoot = await resolveRepoRoot(workspaceDir);
  if (!repoRoot) {
    return {
      workspaceDir,
      isGitRepo: false,
      repoRoot: null,
      currentBranch: null,
      isDirty: false,
      ahead: 0,
      behind: 0,
      changedCount: 0,
      stagedCount: 0,
      unstagedCount: 0,
      untrackedCount: 0,
      files: [],
    };
  }

  const { stdout } = await runGit(repoRoot, ["status", "--porcelain=v1", "-b"]);
  const lines = stdout.split(/\r?\n/).filter(Boolean);
  const branch = parseBranchLine(lines[0] || "##");
  const files = lines.slice(1).map(parseStatusLine).filter((file): file is DesktopWorkspaceGitFile => Boolean(file));
  const stagedCount = files.filter((file) => file.status[0] !== " " && file.status[0] !== "?").length;
  const unstagedCount = files.filter((file) => file.status[1] !== " " && file.status[1] !== "?").length;
  const untrackedCount = files.filter((file) => file.status === "??").length;

  return {
    workspaceDir,
    isGitRepo: true,
    repoRoot,
    currentBranch: branch.branch,
    isDirty: files.length > 0,
    ahead: branch.ahead,
    behind: branch.behind,
    changedCount: files.length,
    stagedCount,
    unstagedCount,
    untrackedCount,
    files,
  };
}

export async function getWorkspaceGitDetails(
  _event: IpcMainInvokeEvent,
  input: { workspacePath: string },
): Promise<DesktopWorkspaceGitDetails> {
  return getWorkspaceGitDetailsForPath(input.workspacePath);
}

export async function commitWorkspaceChanges(
  _event: IpcMainInvokeEvent,
  input: CommitWorkspaceChangesInput,
): Promise<WorkspaceGitMutationResult> {
  const message = input.message.trim();
  if (!message) {
    throw new Error("Commit message is required.");
  }
  const before = await getWorkspaceGitDetailsForPath(input.workspacePath);
  if (!before.isGitRepo || !before.repoRoot) {
    throw new Error("Workspace is not a Git repository.");
  }
  if (!before.isDirty) {
    throw new Error("There are no local changes to commit.");
  }

  await runGit(before.repoRoot, ["add", "-A"]);
  const commit = await runGit(before.repoRoot, ["commit", "-m", message]);
  const status = await getWorkspaceGitDetailsForPath(input.workspacePath);
  return {
    output: `${commit.stdout}${commit.stderr}`.trim(),
    status,
  };
}

export async function pushWorkspaceBranch(
  _event: IpcMainInvokeEvent,
  input: PushWorkspaceBranchInput,
): Promise<WorkspaceGitMutationResult> {
  const before = await getWorkspaceGitDetailsForPath(input.workspacePath);
  if (!before.isGitRepo || !before.repoRoot) {
    throw new Error("Workspace is not a Git repository.");
  }

  const push = await runGit(before.repoRoot, ["push"]);
  const status = await getWorkspaceGitDetailsForPath(input.workspacePath);
  return {
    output: `${push.stdout}${push.stderr}`.trim(),
    status,
  };
}
