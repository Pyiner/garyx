import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

const bannerUrl = new URL("./components/RateLimitBanner.tsx", import.meta.url);

test("quota recovery Continue surfaces terminal, compatibility, and request failures", async () => {
  const source = await readFile(bannerUrl, "utf8");

  assert.doesNotMatch(
    source,
    /\.catch\(\(\) => \{\}\)/,
    "Continue failures must never be swallowed",
  );
  assert.match(source, /result\.status === "settled"/);
  assert.match(source, /result\.status === "unsupported"/);
  assert.match(source, /setRecoveryError\(/);
});

test("quota recovery card resolves its provider name through the shared helper", async () => {
  const source = await readFile(bannerUrl, "utf8");
  assert.doesNotMatch(source, /normalizedProvider === "claude_code"\s*\?\s*"Claude Code"/);
  assert.match(source, /sharedProviderLabel\(normalizedProvider\)/);
});
