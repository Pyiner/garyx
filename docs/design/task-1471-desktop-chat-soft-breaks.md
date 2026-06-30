# TASK-1471 Desktop Chat Soft Breaks

## Goal

Garyx Mac desktop chat should render a single newline in message markdown as a
visible line break for both user and assistant messages:

- `第一行\n第二行` renders as `第一行<br>第二行`.
- Existing markdown structure remains intact: hard breaks, blank-line
  paragraphs, lists, fenced code, and inline code keep their current semantics.

## Current Evidence

- Desktop chat text enters `RichMessageText`, which preprocesses message text
  with `prepareMessageMarkdown(...)`, then renders it through `<Streamdown
  mode="streaming" plugins={{ cjk, code: garyxCodePlugin }}>`.
  Evidence:
  `desktop/garyx-desktop/src/renderer/src/message-rich-text.tsx:125-128`
  and `:171-183`.
- User and assistant messages share this markdown renderer; the `tone` only
  selects `message-rich-default` or `message-rich-assistant` CSS classes.
  Evidence: `message-rich-text.tsx:110` and `:170`.
- The current chat call site does not pass `remarkBreaks` or any
  `remarkPlugins` override. Streamdown therefore keeps CommonMark soft line
  endings as text newlines, which CSS `white-space: normal` collapses visually.
  Evidence: `message-rich-text.tsx:171-180`,
  `desktop/garyx-desktop/src/renderer/src/styles.css:7952-7960`.
- A deterministic headless SSR repro now exists at
  `desktop/garyx-desktop/src/renderer/src/message-rich-text-linebreaks.test.mjs`.
  Current RED result:
  `node --experimental-strip-types --test src/renderer/src/message-rich-text-linebreaks.test.mjs`
  fails the user and assistant single-newline cases with rendered HTML like
  `<p>第一行\n第二行</p>` and no `<br>`. The hard-break, blank paragraph, list,
  and fenced-code controls already pass.
- `WorkflowRunsPanel` already uses `remarkBreaks` with `ReactMarkdown`, which is
  the local precedent for this product behavior in markdown-like desktop output.
  Evidence:
  `desktop/garyx-desktop/src/renderer/src/app-shell/components/WorkflowRunsPanel.tsx:3-5`,
  `:481-485`, and `:1478-1481`.

## Streamdown Extension Point

Streamdown v2.5.0 supports two different extension surfaces:

- `plugins?: PluginConfig` for Streamdown-specific objects such as `cjk`,
  `code`, `math`, and `mermaid`.
- `remarkPlugins?: PluggableList` as a top-level prop inherited from its
  `Options` interface.

Evidence from installed Streamdown v2.5.0:

- `node_modules/streamdown/dist/index.d.ts:65-73` declares top-level
  `remarkPlugins`.
- `node_modules/streamdown/dist/index.d.ts:213-220` declares the separate
  `PluginConfig` shape for `plugins`.
- `node_modules/streamdown/dist/index.d.ts:410-427` shows `StreamdownProps`
  includes both `remarkPlugins` and `plugins`.
- The bundled implementation defaults `remarkPlugins` to Streamdown's default
  GFM/code-meta set, then composes `cjk.remarkPluginsBefore`, the supplied
  `remarkPlugins`, and `cjk.remarkPluginsAfter`.

Design choice:

- Add `remark-breaks` through top-level `remarkPlugins`, not inside
  `plugins`.
- Preserve Streamdown defaults by importing `defaultRemarkPlugins` from
  `streamdown` and passing `[
  ...Object.values(defaultRemarkPlugins),
  remarkBreaks,
  ]`.
- Keep `plugins={{ cjk, code: garyxCodePlugin }}` unchanged so CJK handling and
  code highlighting keep their existing Streamdown plugin path.

## Root Cause D: User Message Storage Shape

Conclusion: desktop composer user messages are already sent and persisted as a
single newline string. No service-side change is needed for the product rule
“press Enter once = show a new line.”

Evidence:

- The textarea passes the raw value into `onComposerChange`; `AppShell` stores
  it in `composerDraftRef.current` without newline normalization.
  Evidence:
  `desktop/garyx-desktop/src/renderer/src/ComposerForm.tsx:1228-1234`,
  `desktop/garyx-desktop/src/renderer/src/app-shell/AppShell.tsx:9897-9905`.
- Before send, `composePromptWithBrowserAnnotations` trims only the prompt
  edges and joins browser annotations with blank lines. Internal single `\n`
  remains single `\n`.
  Evidence: `AppShell.tsx:949-955`, `:8750-8782`, and `:9075-9082`.
- The desktop main process serializes `message: input.message` unchanged for
  both `/api/chat/start` and `/api/chat/stream-input`.
  Evidence: `desktop/garyx-desktop/src/main/gary-client.ts:5915-5957` and
  `:5971-6000`.
- Gateway `/api/chat/start` uses `prepared.effective_message` as the bridge
  `AgentRunRequest` message; slash-command resolution falls back to
  `req.message.clone()` with no newline rewrite.
  Evidence: `garyx-gateway/src/chat.rs:298-375`,
  `garyx-gateway/src/application/chat/prepare.rs:101-106`,
  `:271-283`.
- Gateway `/api/chat/stream-input` similarly resolves slash commands, then
  passes `&effective_message` to `bridge.add_streaming_input`.
  Evidence: `garyx-gateway/src/application/chat/control.rs:34-50`.
- Bridge persistence builds the transcript user record from `run.user_message`;
  plain text without attachments becomes `Value::String(user_message.to_owned())`.
  Evidence: `garyx-bridge/src/multi_provider/persistence.rs:118-124`,
  `:1020-1023`, and `garyx-models/src/provider.rs:904-920`.

Because single newline survives through send and persistence, renderer markdown
configuration is the correct and sufficient fix. Service code should not be
changed.

## Implementation Plan

1. Update the chat Streamdown configuration in `message-rich-text.tsx`:
   pass a stable shared `CHAT_MESSAGE_REMARK_PLUGINS` array via
   `<Streamdown remarkPlugins={...}>`.
2. Put `CHAT_MESSAGE_REMARK_PLUGINS` in a no-JSX TypeScript module, e.g.
   `desktop/garyx-desktop/src/renderer/src/message-rich-text-plugins.ts`.
   That module imports `defaultRemarkPlugins` from `streamdown` and
   `remarkBreaks` from `remark-breaks`, then exports:
   `[
   ...Object.values(defaultRemarkPlugins),
   remarkBreaks,
   ]`.
   Do not put this shared array in `message-rich-text.tsx`: the headless
   renderer tests use `node --experimental-strip-types --test`, and that runner
   cannot import `.tsx` files.
3. Keep CSS `white-space: normal`; do not switch message blocks to
   `pre-wrap`.
4. Update the headless test so it imports that `.ts` shared chat markdown plugin
   array instead of duplicating the pre-fix config. This proves the markdown
   configuration itself while avoiding a `.tsx` import. The packaged-app
   screenshot step proves production `RichMessageText` is wired to the shared
   config.
5. Add the new test file to `npm run test:unit`.
6. Add `remark-breaks` as a direct desktop dependency because the code imports
   it directly. It is already available transitively through Streamdown's
   dependency tree and is already directly imported by `WorkflowRunsPanel`, but
   direct declaration keeps the dependency contract explicit.

## Validation Plan

- RED already captured:
  `node --experimental-strip-types --test src/renderer/src/message-rich-text-linebreaks.test.mjs`
  fails user and assistant soft-break cases because the HTML has no `<br>`.
- GREEN after implementation:
  same focused command passes all cases:
  user soft break, assistant soft break, hard break, blank-line paragraphs,
  lists, fenced code, and inline code.
- Run desktop focused/full validation:
  `npm run test:unit`
  and `npm run build:ui` in `desktop/garyx-desktop`.
- Packaged app proof:
  `npm run dist:dir`, quit stale `Garyx`, open installed app, attach via CDP,
  and capture screenshots showing one user message and one assistant message
  with single-newline visual breaks.

## Risks

- Passing `remarkPlugins` replaces Streamdown's default remark plugin list, so
  the implementation must include `defaultRemarkPlugins`; otherwise GFM/code
  metadata behavior could regress.
- Preserve `Object.values(defaultRemarkPlugins)` order. Streamdown v2.5.0's
  default keys are `gfm` then `codeMeta`; do not sort or reorder the array.
- `remarkBreaks` affects text nodes broadly, so regression tests must keep
  coverage for lists and code regions.
- The headless test cannot directly import `message-rich-text.tsx` under the
  repo's `node --experimental-strip-types --test` runner. The shared `.ts`
  plugin module plus packaged-app CDP screenshots are the intended coverage
  split.
- End-to-end proof needs an installed-app run, not only Vite/dev-server output.

## Design Review Rework

Initial design review returned `NEEDS-REWORK` on one narrow blocker: the first
version placed the shared plugin array in `message-rich-text.tsx` and then
planned for the `.mjs` Node test to import it. That would fail because the
desktop unit runner uses `node --experimental-strip-types --test`, which does
not load `.tsx` files. The plan above fixes this by moving the shared plugin
array into `message-rich-text-plugins.ts`.

Re-review result: PASS. The corrected `.ts` shared plugin module was probed
under the same strip-types runner and loaded successfully, preserving
Streamdown defaults plus `remarkBreaks`.
