import test from 'node:test';
import assert from 'node:assert/strict';

import {
  CAPSULE_THUMBNAIL_DEVICE_WIDTH,
  CAPSULE_THUMBNAIL_SCHEMA_VERSION,
  capsuleThumbnailFillScript,
  capsuleThumbnailScrollbarHidingStyle,
  capsuleThumbnailStorageToken,
  ensureMobileViewport,
  evictingStaleSchemaTokens,
  fillTransform,
  hideScrollbars,
  prepareThumbnailHtml,
} from './capsule-thumbnail-html.ts';

const GALLERY = { aspectWidth: 16, aspectHeight: 10 };
const CHAT = { aspectWidth: 16, aspectHeight: 9 };

// --- device width + render schema --------------------------------------------

test('renders at the device logical width (390), matching iOS', () => {
  assert.equal(CAPSULE_THUMBNAIL_DEVICE_WIDTH, 390);
});

test('current render schema is bumped past the original wide render', () => {
  assert.ok(CAPSULE_THUMBNAIL_SCHEMA_VERSION >= 2);
});

// --- storage token embeds the schema so a bump invalidates old caches --------

test('storage token embeds rendition and schema version', () => {
  const v = CAPSULE_THUMBNAIL_SCHEMA_VERSION;
  assert.equal(capsuleThumbnailStorageToken('cap', 3, GALLERY), `cap.r3.16x10.s${v}`);
  assert.equal(capsuleThumbnailStorageToken('cap', 3, CHAT), `cap.r3.16x9.s${v}`);
});

test('a schema bump changes the token so old renders are never read back', () => {
  const old = capsuleThumbnailStorageToken('cap', 3, GALLERY, 1);
  const next = capsuleThumbnailStorageToken('cap', 3, GALLERY, 2);
  assert.equal(old, 'cap.r3.16x10.s1');
  assert.notEqual(old, next);
});

test('storage token trims the id', () => {
  assert.equal(
    capsuleThumbnailStorageToken('  cap  ', 1, GALLERY),
    `cap.r1.16x10.s${CAPSULE_THUMBNAIL_SCHEMA_VERSION}`,
  );
});

// --- stale-schema purge (cache invalidation on warm) -------------------------

test('evicts tokens from an older schema / legacy tokens, keeps current', () => {
  const v = CAPSULE_THUMBNAIL_SCHEMA_VERSION;
  const current = capsuleThumbnailStorageToken('cap', 3, GALLERY); // .s{v}
  const entries = [
    { token: 'cap.r3.16x10', currentToken: current }, // legacy: no schema suffix
    { token: 'cap.r3.16x10.s1', currentToken: current }, // older schema
    { token: current, currentToken: current }, // current schema
  ];
  const { keep, evict } = evictingStaleSchemaTokens(entries);
  assert.deepEqual(keep, [current]);
  assert.deepEqual(new Set(evict), new Set(['cap.r3.16x10', 'cap.r3.16x10.s1']));
});

test('keeps everything when all tokens are current schema', () => {
  const g = capsuleThumbnailStorageToken('a', 1, GALLERY);
  const c = capsuleThumbnailStorageToken('a', 1, CHAT);
  const { keep, evict } = evictingStaleSchemaTokens([
    { token: g, currentToken: g },
    { token: c, currentToken: c },
  ]);
  assert.equal(evict.length, 0);
  assert.deepEqual(new Set(keep), new Set([g, c]));
});

// --- horizontal content-fill transform ---------------------------------------

test('content that already fills needs no transform', () => {
  assert.equal(fillTransform(0, 390, 390), null);
  assert.equal(fillTransform(0.4, 389.3, 390), null); // sub-pixel slack
});

test('narrow centered content (max-width:300 → 364px in 390) scales to fill flush-left', () => {
  const t = fillTransform(13, 364, 390);
  assert.ok(t);
  assert.ok(Math.abs(t.scale - 390 / 364) < 1e-9);
  assert.ok(Math.abs(t.translateX - -13 * (390 / 364)) < 1e-9);
  // Maps content edges onto [0, 390].
  assert.ok(Math.abs(13 * t.scale + t.translateX - 0) < 1e-6);
  assert.ok(Math.abs((13 + 364) * t.scale + t.translateX - 390) < 1e-6);
});

test('any left gutter triggers a transform', () => {
  assert.ok(fillTransform(8, 382, 390));
});

test('guards against invalid measurements', () => {
  assert.equal(fillTransform(0, 0, 390), null);
  assert.equal(fillTransform(0, 390, 0), null);
});

// --- viewport injection (unchanged behavior, now from the shared module) -----

test('injects device-width viewport when absent', () => {
  const out = ensureMobileViewport('<head><title>x</title></head><body>b</body>');
  assert.match(out, /<head><meta name="viewport"/);
  assert.match(out, /width=device-width/);
});

test('leaves an author-declared viewport untouched', () => {
  const html = '<head><meta name="viewport" content="width=320"></head><body>b</body>';
  assert.equal(ensureMobileViewport(html), html);
});

// --- scrollbar hiding during capture (#TASK-1478) ----------------------------

test('scrollbar-hiding style hides webkit (root + inner) and firefox scrollbars', () => {
  assert.match(capsuleThumbnailScrollbarHidingStyle, /::-webkit-scrollbar\s*{[^}]*display:\s*none/);
  assert.match(capsuleThumbnailScrollbarHidingStyle, /scrollbar-width:\s*none/);
  // !important so an author's own scrollbar styling can't re-enable it.
  assert.match(capsuleThumbnailScrollbarHidingStyle, /display:\s*none\s*!important/);
});

test('hideScrollbars injects the style after <head> open', () => {
  const out = hideScrollbars('<head><title>x</title></head><body>b</body>');
  assert.match(out, /<head><style id="garyx-thumbnail-scrollbar-hide"/);
  assert.match(out, /::-webkit-scrollbar/);
  assert.match(out, /<title>x<\/title>/); // original markup preserved
});

test('hideScrollbars prepends when there is no <head>', () => {
  const out = hideScrollbars('<main>demo</main>');
  assert.ok(out.startsWith('<style id="garyx-thumbnail-scrollbar-hide"'));
  assert.ok(out.endsWith('<main>demo</main>'));
});

test('hideScrollbars is idempotent (no double injection)', () => {
  const once = hideScrollbars('<head></head><body>b</body>');
  assert.equal(hideScrollbars(once), once);
  assert.equal(once.match(/garyx-thumbnail-scrollbar-hide/g).length, 1);
});

test('prepareThumbnailHtml guarantees both a device-width viewport and hidden scrollbars', () => {
  const out = prepareThumbnailHtml('<head><title>x</title></head><body>b</body>');
  assert.match(out, /width=device-width/);
  assert.match(out, /::-webkit-scrollbar/);
});

test('prepareThumbnailHtml respects an author viewport but still hides scrollbars', () => {
  const html = '<head><meta name="viewport" content="width=320"></head><body>b</body>';
  const out = prepareThumbnailHtml(html);
  assert.match(out, /width=320/); // author viewport untouched
  assert.ok(!out.includes('width=device-width'));
  assert.match(out, /garyx-thumbnail-scrollbar-hide/); // scrollbars still hidden
});

// --- the injected JS mirrors fillTransform -----------------------------------

test('fill script carries the same arithmetic as fillTransform', () => {
  assert.match(capsuleThumbnailFillScript, /window\.innerWidth/);
  assert.match(capsuleThumbnailFillScript, /getBoundingClientRect/);
  assert.match(capsuleThumbnailFillScript, /vw \/ width/);
  assert.match(capsuleThumbnailFillScript, /scale <= 1\.005 && left <= 1/);
  assert.match(capsuleThumbnailFillScript, /translateX\('? \+ \(-left \* scale\)/);
  assert.match(capsuleThumbnailFillScript, /transformOrigin = 'top left'/);
});
