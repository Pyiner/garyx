import { execFile as execFileCallback } from "node:child_process";
import { constants } from "node:fs";
import { access, mkdtemp, readFile, rm } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";

import type {
  GenerateCustomAgentAvatarInput,
  GenerateCustomAgentAvatarResult,
} from "@shared/contracts";

import { resolveDesktopConfigPath } from "./config-paths";

const execFile = promisify(execFileCallback);

type ToolImageJsonPayload = {
  ok?: boolean;
  path?: string;
  media_type?: string | null;
  mediaType?: string | null;
};

function avatarName(input: GenerateCustomAgentAvatarInput): string {
  return input.displayName.trim() || input.agentId?.trim() || "Agent";
}

function buildAgentAvatarPrompt(input: GenerateCustomAgentAvatarInput): string {
  const name = JSON.stringify(avatarName(input));
  return [
    `Create a square app avatar for an AI agent named ${name}.`,
    "Style: polished macOS developer tool icon, centered abstract agent mark, crisp silhouette, subtle dimensional lighting, high contrast at 32px.",
    "Do not include text, letters, badges, logos, screenshots, people, or UI chrome.",
  ].join("\n");
}

async function executableExists(path: string): Promise<boolean> {
  try {
    await access(path, constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

async function candidateGaryxCommands(): Promise<string[]> {
  const candidates = [
    process.env.GARYX_BIN?.trim(),
    join(process.cwd(), "target", "release", "garyx"),
    join(process.cwd(), "target", "debug", "garyx"),
    join(process.cwd(), "..", "..", "target", "release", "garyx"),
    join(process.cwd(), "..", "..", "target", "debug", "garyx"),
    join(homedir(), ".cargo", "bin", "garyx"),
    "/opt/homebrew/bin/garyx",
    "/usr/local/bin/garyx",
    "garyx",
  ].filter((candidate): candidate is string => Boolean(candidate));

  const uniqueCandidates = Array.from(new Set(candidates));
  const resolved = await Promise.all(
    uniqueCandidates.map(async (candidate) => {
      if (!candidate.includes("/")) {
        return candidate;
      }
      return (await executableExists(candidate)) ? candidate : null;
    }),
  );
  return resolved.filter((candidate): candidate is string => Boolean(candidate));
}

function parseToolImageJson(stdout: string): ToolImageJsonPayload {
  const trimmed = stdout.trim();
  const jsonStart = trimmed.indexOf("{");
  const jsonText = jsonStart >= 0 ? trimmed.slice(jsonStart) : trimmed;
  return JSON.parse(jsonText) as ToolImageJsonPayload;
}

function mediaTypeForPath(path: string, fallback?: string | null): string {
  if (fallback?.trim()) {
    return fallback.trim();
  }
  if (path.endsWith(".jpg") || path.endsWith(".jpeg")) {
    return "image/jpeg";
  }
  if (path.endsWith(".webp")) {
    return "image/webp";
  }
  return "image/png";
}

async function runToolImage(command: string, prompt: string, outputPath: string) {
  const configPath = await resolveDesktopConfigPath();
  const extraPath = [
    "/opt/homebrew/bin",
    "/usr/local/bin",
    join(homedir(), ".cargo", "bin"),
  ].join(":");
  const pathValue = [process.env.PATH, extraPath].filter(Boolean).join(":");
  return execFile(
    command,
    [
      "--config",
      configPath,
      "tool",
      "image",
      prompt,
      "--output",
      outputPath,
      "--json",
      "--timeout",
      "600",
    ],
    {
      encoding: "utf8",
      env: {
        ...process.env,
        PATH: pathValue,
      },
      maxBuffer: 1024 * 1024,
    },
  ) as Promise<{ stdout: string; stderr: string }>;
}

export async function generateCustomAgentAvatar(
  input: GenerateCustomAgentAvatarInput,
): Promise<GenerateCustomAgentAvatarResult> {
  const prompt = buildAgentAvatarPrompt(input);
  const tempDir = await mkdtemp(join(tmpdir(), "garyx-agent-avatar-"));
  const outputPath = join(tempDir, "avatar.png");
  const commands = await candidateGaryxCommands();
  let lastError: unknown = null;

  try {
    for (const command of commands) {
      try {
        const { stdout } = await runToolImage(command, prompt, outputPath);
        const payload = parseToolImageJson(stdout);
        const generatedPath = payload.path?.trim() || outputPath;
        const bytes = await readFile(generatedPath);
        const mediaType = mediaTypeForPath(generatedPath, payload.media_type || payload.mediaType);
        return {
          avatarDataUrl: `data:${mediaType};base64,${bytes.toString("base64")}`,
          mediaType,
        };
      } catch (error) {
        lastError = error;
      }
    }

    const detail = lastError instanceof Error && lastError.message.trim()
      ? ` ${lastError.message.trim()}`
      : "";
    throw new Error(`Unable to generate avatar with garyx tool image.${detail}`);
  } finally {
    await rm(tempDir, { recursive: true, force: true });
  }
}
