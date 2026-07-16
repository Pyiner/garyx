import { Buffer } from "node:buffer";

const IMAGE_FILE_EXTENSIONS: Readonly<Record<string, string>> = {
  "image/apng": "png",
  "image/avif": "avif",
  "image/bmp": "bmp",
  "image/gif": "gif",
  "image/heic": "heic",
  "image/heif": "heif",
  "image/jpeg": "jpg",
  "image/png": "png",
  "image/svg+xml": "svg",
  "image/tiff": "tiff",
  "image/vnd.microsoft.icon": "ico",
  "image/webp": "webp",
  "image/x-icon": "ico",
};

export interface DecodedImageDataUrl {
  bytes: Buffer;
  extension: string;
  mediaType: string;
}

function normalizeMediaType(mediaType: string): string {
  return mediaType.split(";", 1)[0]?.trim().toLowerCase() || "";
}

export function inferImageFileExtension(mediaType: string): string {
  const normalized = normalizeMediaType(mediaType);
  if (!normalized.startsWith("image/")) {
    throw new Error("data URL must contain an image media type");
  }

  const knownExtension = IMAGE_FILE_EXTENSIONS[normalized];
  if (knownExtension) {
    return knownExtension;
  }

  const subtype = normalized.slice("image/".length);
  const inferred = subtype
    .replace(/^x-/, "")
    .split("+", 1)[0]
    ?.replace(/[^a-z0-9]/g, "");
  return inferred || "img";
}

export function buildImageSaveFileName(
  suggestedName: string | undefined,
  mediaType: string,
): string {
  const extension = inferImageFileExtension(mediaType);
  const finalPathSegment = suggestedName?.trim().split(/[\\/]/).at(-1) || "";
  const sanitized = finalPathSegment
    .replace(/[\u0000-\u001f<>:"/\\|?*]/g, "-")
    .replace(/[.\s]+$/g, "")
    .trim();
  const stem = sanitized
    .replace(/\.[a-z0-9]{1,10}$/i, "")
    .replace(/[.\s]+$/g, "")
    .trim();
  return `${stem || "image"}.${extension}`;
}

export function decodeImageDataUrl(dataUrl: string): DecodedImageDataUrl {
  const normalized = dataUrl.trim();
  const commaIndex = normalized.indexOf(",");
  if (!normalized.startsWith("data:") || commaIndex < 0) {
    throw new Error("image source must be a data URL");
  }

  const metadata = normalized.slice("data:".length, commaIndex);
  const [rawMediaType = "", ...parameters] = metadata.split(";");
  if (parameters.at(-1)?.trim().toLowerCase() !== "base64") {
    throw new Error("image data URL must use base64 encoding");
  }

  const mediaType = normalizeMediaType(rawMediaType);
  const extension = inferImageFileExtension(mediaType);
  const encoded = normalized.slice(commaIndex + 1).replace(/\s/g, "");
  if (!encoded || !/^[a-z0-9+/]*={0,2}$/i.test(encoded)) {
    throw new Error("image data URL contains invalid base64");
  }

  const withoutPadding = encoded.replace(/=+$/g, "");
  if (
    withoutPadding.length % 4 === 1 ||
    (encoded.includes("=") && encoded.length % 4 !== 0)
  ) {
    throw new Error("image data URL contains invalid base64");
  }

  const padded = withoutPadding.padEnd(
    Math.ceil(withoutPadding.length / 4) * 4,
    "=",
  );
  return {
    bytes: Buffer.from(padded, "base64"),
    extension,
    mediaType,
  };
}
