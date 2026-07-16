import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import test from "node:test";

import * as esbuild from "esbuild";

const bundled = await esbuild.build({
  entryPoints: ["src/main/image-save.ts"],
  bundle: true,
  format: "esm",
  platform: "node",
  write: false,
});
const imageSave = await import(
  `data:text/javascript;base64,${Buffer.from(bundled.outputFiles[0].text).toString("base64")}`
);
const {
  buildImageSaveFileName,
  decodeImageDataUrl,
  inferImageFileExtension,
} = imageSave;

test("image media types map to useful save extensions", () => {
  assert.equal(inferImageFileExtension("image/jpeg"), "jpg");
  assert.equal(inferImageFileExtension(" IMAGE/PNG "), "png");
  assert.equal(inferImageFileExtension("image/svg+xml"), "svg");
  assert.equal(inferImageFileExtension("image/webp"), "webp");
  assert.equal(inferImageFileExtension("image/x-custom"), "custom");
  assert.throws(
    () => inferImageFileExtension("text/plain"),
    /image media type/,
  );
});

test("suggested image names are sanitized and receive the media type extension", () => {
  assert.equal(
    buildImageSaveFileName("photos/preview.jpeg", "image/png"),
    "preview.png",
  );
  assert.equal(
    buildImageSaveFileName("diagram:final.png", "image/svg+xml"),
    "diagram-final.svg",
  );
  assert.equal(buildImageSaveFileName(undefined, "image/webp"), "image.webp");
});

test("base64 image data URLs decode to the original bytes", () => {
  const original = Buffer.from([0, 1, 2, 3, 127, 128, 254, 255]);
  const decoded = decodeImageDataUrl(
    `data:image/png;base64,\n${original.toString("base64")}\n`,
  );

  assert.equal(decoded.mediaType, "image/png");
  assert.equal(decoded.extension, "png");
  assert.deepEqual(decoded.bytes, original);
});

test("image data URL decoding rejects non-images and malformed base64", () => {
  assert.throws(
    () => decodeImageDataUrl("data:text/plain;base64,aGVsbG8="),
    /image media type/,
  );
  assert.throws(
    () => decodeImageDataUrl("data:image/png,not-base64"),
    /base64 encoding/,
  );
  assert.throws(
    () => decodeImageDataUrl("data:image/png;base64,a==="),
    /invalid base64/,
  );
});
