import { nativeImage } from "electron";

import type {
  DesktopSettings,
  GenerateCustomAgentAvatarInput,
  GenerateCustomAgentAvatarResult,
} from "@shared/contracts";

import { requestJson } from "./gary-client";

const AVATAR_IMAGE_SIZE = 256;
const AVATAR_PNG_MAX_BYTES = 450 * 1024;
const AVATAR_JPEG_QUALITY = 88;
const TOOL_IMAGE_TIMEOUT_SECS = 600;
const TOOL_IMAGE_REQUEST_TIMEOUT_MS = (TOOL_IMAGE_TIMEOUT_SECS + 30) * 1000;

type ToolImagePayload = {
  ok?: boolean;
  data_base64?: string | null;
  dataBase64?: string | null;
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

export async function generateCustomAgentAvatar(
  settings: DesktopSettings,
  input: GenerateCustomAgentAvatarInput,
): Promise<GenerateCustomAgentAvatarResult> {
  const prompt = buildAgentAvatarPrompt(input);
  const payload = await requestJson<ToolImagePayload>(settings, "/api/tools/image", {
    method: "POST",
    signal: AbortSignal.timeout(TOOL_IMAGE_REQUEST_TIMEOUT_MS),
    body: JSON.stringify({
      prompt,
      timeout_secs: TOOL_IMAGE_TIMEOUT_SECS,
    }),
  });
  const encoded = (payload.data_base64 || payload.dataBase64 || "").trim();
  if (!encoded) {
    throw new Error("Image generation API did not return image data.");
  }
  const mediaType = (payload.media_type || payload.mediaType || "image/png").trim() || "image/png";
  return normalizeAvatarImage(Buffer.from(encoded, "base64"), mediaType);
}
