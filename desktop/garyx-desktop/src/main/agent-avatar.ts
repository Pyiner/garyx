import { nativeImage } from "electron";

import type {
  DesktopSettings,
  GeneratedCustomAgentAvatar,
  GenerateCustomAgentAvatarInput,
  GenerateCustomAgentAvatarResult,
} from "@shared/contracts";
import { buildAgentAvatarPrompt } from "@shared/agent-avatar-prompt";

import { GatewayRequestError, requestJson } from "./gary-client";
import { AgentAvatarRequestManager } from "./agent-avatar-request-manager";

const AVATAR_IMAGE_SIZE = 256;
const AVATAR_PNG_MAX_BYTES = 450 * 1024;
const AVATAR_JPEG_QUALITY = 88;
const TOOL_IMAGE_TIMEOUT_SECS = 600;
const TOOL_IMAGE_REQUEST_TIMEOUT_MS = (TOOL_IMAGE_TIMEOUT_SECS + 30) * 1000;
const avatarRequests = new AgentAvatarRequestManager();

type ToolImagePayload = {
  ok?: boolean;
  data_base64?: string | null;
  dataBase64?: string | null;
  media_type?: string | null;
  mediaType?: string | null;
};

function avatarDataUrl(
  bytes: Buffer,
  mediaType: string,
): GeneratedCustomAgentAvatar {
  return {
    avatarDataUrl: `data:${mediaType};base64,${bytes.toString("base64")}`,
    mediaType,
  };
}

function normalizeAvatarImage(
  bytes: Buffer,
  _fallbackMediaType: string,
): GeneratedCustomAgentAvatar {
  const image = nativeImage.createFromBuffer(bytes);
  if (image.isEmpty()) {
    throw new Error("Generated avatar image could not be decoded.");
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
  return avatarRequests.run(
    input.requestId,
    TOOL_IMAGE_REQUEST_TIMEOUT_MS,
    async ({ signal, userSignal, timeoutSignal }) => {
      try {
        const prompt = buildAgentAvatarPrompt(input);
        const payload = await requestJson<ToolImagePayload>(settings, "/api/tools/image", {
          method: "POST",
          signal,
          body: JSON.stringify({
            prompt,
            timeout_secs: TOOL_IMAGE_TIMEOUT_SECS,
          }),
        });
        const encoded = (payload.data_base64 || payload.dataBase64 || "").trim();
        if (!encoded) {
          return avatarFailure(
            "unusable",
            "The generated image couldn’t be used.",
          );
        }
        const mediaType = (payload.media_type || payload.mediaType || "image/png").trim() || "image/png";
        try {
          const avatar = normalizeAvatarImage(Buffer.from(encoded, "base64"), mediaType);
          return { status: "success", ...avatar };
        } catch {
          return avatarFailure(
            "unusable",
            "The generated image couldn’t be used.",
          );
        }
      } catch (error) {
        if (userSignal.aborted) {
          return { status: "cancelled" };
        }
        if (timeoutSignal.aborted) {
          return avatarFailure("timeout", "Avatar generation took too long.");
        }
        if (error instanceof GatewayRequestError) {
          if (error.status === 504) {
            return avatarFailure("timeout", "Avatar generation took too long.");
          }
          return avatarFailure(
            "provider",
            "The image provider couldn’t generate an avatar.",
          );
        }
        if (error instanceof TypeError) {
          return avatarFailure("unreachable", "Couldn’t reach the gateway.");
        }
        return avatarFailure(
          "provider",
          "The image provider couldn’t generate an avatar.",
        );
      }
    },
  );
}

export function cancelCustomAgentAvatarGeneration(requestId: string): boolean {
  return avatarRequests.cancel(requestId);
}

function avatarFailure(
  category: "unreachable" | "timeout" | "provider" | "unusable",
  message: string,
): GenerateCustomAgentAvatarResult {
  return { status: "failure", category, message };
}
