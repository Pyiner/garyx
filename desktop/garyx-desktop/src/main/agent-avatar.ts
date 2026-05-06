import { execFile as execFileCallback } from "node:child_process";
import { constants } from "node:fs";
import { access, mkdtemp, readFile, rm } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";

import { nativeImage } from "electron";

import type {
  GenerateCustomAgentAvatarInput,
  GenerateCustomAgentAvatarResult,
} from "@shared/contracts";

import { resolveDesktopConfigPath } from "./config-paths";

const execFile = promisify(execFileCallback);
const AVATAR_IMAGE_SIZE = 256;
const AVATAR_PNG_MAX_BYTES = 450 * 1024;
const AVATAR_JPEG_QUALITY = 88;

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
  const stylePrompt = input.stylePrompt?.trim()
    || "minimal vector glyph, simple geometry, balanced negative space, one confident accent color";
  return [
    `Create a square app avatar for an AI agent named ${name}.`,
    `Visual style: ${stylePrompt}.`,
    "Composition: one centered abstract agent mark, clean silhouette, readable at 32px, restrained palette, polished macOS developer-tool finish.",
    "Do not include text, letters, watermarks, screenshots, people, or UI chrome.",
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

function avatarDataUrl(
  bytes: Buffer,
  mediaType: string,
): GenerateCustomAgentAvatarResult {
  return {
    avatarDataUrl: `data:${mediaType};base64,${bytes.toString("base64")}`,
    mediaType,
  };
}

function normalizeAvatarImage(
  bytes: Buffer,
  fallbackMediaType: string,
): GenerateCustomAgentAvatarResult {
  const image = nativeImage.createFromBuffer(bytes);
  if (image.isEmpty()) {
    if (bytes.length > AVATAR_PNG_MAX_BYTES) {
      throw new Error("Generated avatar image could not be resized.");
    }
    return avatarDataUrl(bytes, fallbackMediaType);
  }

  const resized = image.resize({
    width: AVATAR_IMAGE_SIZE,
    height: AVATAR_IMAGE_SIZE,
    quality: "best",
  });
  const png = resized.toPNG();
  if (png.length <= AVATAR_PNG_MAX_BYTES) {
    return avatarDataUrl(png, "image/png");
  }

  return avatarDataUrl(resized.toJPEG(AVATAR_JPEG_QUALITY), "image/jpeg");
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
        return normalizeAvatarImage(bytes, mediaType);
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
