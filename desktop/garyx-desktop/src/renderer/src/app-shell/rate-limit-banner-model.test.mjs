import test from "node:test";
import assert from "node:assert/strict";

import {
  deriveRateLimitBannerState,
  formatRemaining,
  formatResetClock,
  messageSegments,
  normalizeRateLimitProvider,
} from "./rate-limit-banner-model.ts";

const NOW = Date.parse("2030-01-01T06:00:00Z");

test("free-form provider prefixes normalize to shared presentation keys", () => {
  assert.deepEqual(
    [
      " claude_sdk ",
      "codex_app_server",
      "Antigravity CLI",
      "agy_local",
      "TRAE",
      "traex_bridge",
      "Grok Build",
      "gemini_cli",
      "custom-provider",
      "",
      null,
    ].map((provider) => normalizeRateLimitProvider(provider)),
    [
      "claude_code",
      "codex_app_server",
      "antigravity",
      "antigravity",
      "traex",
      "traex",
      "grok_build",
      "gemini",
      null,
      null,
      null,
    ],
  );
});

test("auto-resend with future reset counts down without Continue", () => {
  const state = deriveRateLimitBannerState({
    resetAtMs: NOW + 330_000,
    nowMs: NOW,
    willAutoResend: true,
    hasMessage: false,
  });
  assert.deepEqual(state, {
    kind: "auto_resend_countdown",
    showContinue: false,
  });
});

test("auto-resend past reset flips to resending", () => {
  const state = deriveRateLimitBannerState({
    resetAtMs: NOW - 5_000,
    nowMs: NOW,
    willAutoResend: true,
    hasMessage: true,
  });
  assert.deepEqual(state, { kind: "resending", showContinue: false });
});

test("auto-resend without reset time stays pending", () => {
  const state = deriveRateLimitBannerState({
    resetAtMs: Number.NaN,
    nowMs: NOW,
    willAutoResend: true,
    hasMessage: true,
  });
  assert.deepEqual(state, { kind: "auto_resend_pending", showContinue: false });
});

test("manual reset countdown offers Continue", () => {
  const state = deriveRateLimitBannerState({
    resetAtMs: NOW + 60_000,
    nowMs: NOW,
    willAutoResend: false,
    hasMessage: false,
  });
  assert.deepEqual(state, { kind: "resets_countdown", showContinue: true });
});

test("manual past reset reports recovered with Continue", () => {
  const state = deriveRateLimitBannerState({
    resetAtMs: NOW - 600_000,
    nowMs: NOW,
    willAutoResend: false,
    hasMessage: true,
  });
  assert.deepEqual(state, { kind: "recovered", showContinue: true });
});

test("no reset falls back to provider message, then plain", () => {
  assert.deepEqual(
    deriveRateLimitBannerState({
      resetAtMs: Number.NaN,
      nowMs: NOW,
      willAutoResend: false,
      hasMessage: true,
    }),
    { kind: "message", showContinue: true },
  );
  assert.deepEqual(
    deriveRateLimitBannerState({
      resetAtMs: Number.NaN,
      nowMs: NOW,
      willAutoResend: false,
      hasMessage: false,
    }),
    { kind: "plain", showContinue: true },
  );
});

test("formatRemaining pads minutes and includes hours only when needed", () => {
  assert.equal(formatRemaining(330_000), "05:30");
  assert.equal(formatRemaining(3_661_000), "1:01:01");
  assert.equal(formatRemaining(-1_000), "00:00");
});

test("formatResetClock shows time only for same-day resets", () => {
  const reset = Date.parse("2030-01-01T14:42:00Z");
  const clock = formatResetClock(reset, NOW, "en");
  // Rendered in the machine-local timezone; same rules as transcripts —
  // assert shape, not a fixed zone-dependent value.
  assert.match(clock, /^\d{2}:\d{2}$/);
});

test("formatResetClock includes the date once the reset is not today", () => {
  const reset = Date.parse("2030-01-08T00:00:00Z");
  const clock = formatResetClock(reset, NOW, "en");
  assert.match(clock, /^[A-Za-z]{3} \d{1,2} \d{2}:\d{2}$/);
});

test("messageSegments linkifies bare URLs verbatim and keeps trailing punctuation", () => {
  const segments = messageSegments(
    "You've hit your usage limit. Visit https://example.com/codex/settings/usage to purchase more credits or try again at 9:42 PM.",
  );
  assert.deepEqual(segments, [
    { kind: "text", text: "You've hit your usage limit. Visit " },
    {
      kind: "link",
      text: "https://example.com/codex/settings/usage",
      url: "https://example.com/codex/settings/usage",
    },
    {
      kind: "text",
      text: " to purchase more credits or try again at 9:42 PM.",
    },
  ]);
});

test("messageSegments keeps sentence punctuation out of a trailing URL", () => {
  for (const punctuation of [".", "!", "?", ";", '."', ")!", "。", "！"]) {
    const segments = messageSegments(`See https://example.com/usage${punctuation}`);
    assert.deepEqual(
      segments,
      [
        { kind: "text", text: "See " },
        {
          kind: "link",
          text: "https://example.com/usage",
          url: "https://example.com/usage",
        },
        { kind: "text", text: punctuation },
      ],
      `punctuation ${JSON.stringify(punctuation)}`,
    );
  }
});

test("messageSegments keeps balanced closing brackets inside the URL", () => {
  const segments = messageSegments(
    "See https://en.wikipedia.org/wiki/Rate_(computing).",
  );
  assert.deepEqual(segments, [
    { kind: "text", text: "See " },
    {
      kind: "link",
      text: "https://en.wikipedia.org/wiki/Rate_(computing)",
      url: "https://en.wikipedia.org/wiki/Rate_(computing)",
    },
    { kind: "text", text: "." },
  ]);
});

test("messageSegments passes plain text through untouched", () => {
  assert.deepEqual(messageSegments("Try again shortly."), [
    { kind: "text", text: "Try again shortly." },
  ]);
});

test("messageSegments treats a punctuation-only URL token as text", () => {
  assert.deepEqual(messageSegments("broken https://... link"), [
    { kind: "text", text: "broken " },
    { kind: "text", text: "https://..." },
    { kind: "text", text: " link" },
  ]);
});
