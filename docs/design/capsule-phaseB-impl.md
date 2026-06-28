# Capsule Phase B Desktop Implementation Plan

Status: approved for implementation by design review #TASK-1415. This phase is desktop-only and assumes Phase A gateway APIs from commit `b519e02c` are present.

## Source design constraints

Implementation follows `docs/design/capsule.md` sections 4-7:

- Renderer must not receive or construct gateway auth. Capsule metadata and HTML are fetched by the main process through the existing authenticated Garyx client and exposed through preload IPC.
- HTML execution must use `<iframe sandbox="allow-scripts" srcDoc={htmlFromIpc} />` only. No `allow-same-origin`, no Electron `webview`, no `WebContentsView`, and no direct `src` pointing at `/serve`.
- Desktop navigation gets a top-level `Capsules` entry after Automation/Dreams and before Tasks, with route `#/capsules` and `ContentView` value `capsules`.
- The new panel owns only desktop composition state; gateway DB/file state remains authoritative.
- Agent/provider presentation must reuse existing desktop identity helpers (`AgentOptionAvatar` / `ProviderAgentIcon` data), not local provider switch tables.

## Existing seams to reuse

- API contracts live in `desktop/garyx-desktop/src/shared/contracts.ts` and are consumed by main, preload, and renderer.
- Authenticated gateway JSON fetch is centralized in `src/main/gary-client.ts` via `requestJson`. I will add a parallel text helper using the same auth/custom-header path.
- Main IPC handlers are registered in `src/main/index.ts` next to Dreams/Tasks handlers, with settings resolved in main before calling `gary-client` methods.
- Preload exposes `window.garyxDesktop` by mirroring `GaryxDesktopApi` methods through `ipcRenderer.invoke`.
- App navigation and saved/deep-link routing live in `app-shell/types.ts`, `app-shell/desktop-route.ts`, `AppLeftRail.tsx`, and `AppShell.tsx`.
- Existing top-level panels (`DreamsPanel`, `TasksPanel`) provide loading/error/empty/header/button class patterns. Capsules will reuse these tokens/styles instead of introducing a new component framework.

## Data and IPC contract

### Shared types (`src/shared/contracts.ts`)

Add:

```ts
export interface DesktopCapsuleSummary {
  id: string;
  title: string;
  description: string;
  threadId?: string | null;
  runId?: string | null;
  agentId?: string | null;
  providerType?: DesktopApiProviderType | string | null;
  htmlSha256: string;
  byteSize: number;
  revision: number;
  createdAt: string;
  updatedAt: string;
}

export interface DesktopCapsulesPage {
  capsules: DesktopCapsuleSummary[];
}

export interface DeleteCapsuleInput {
  capsuleId: string;
}
```

Extend `GaryxDesktopApi`:

```ts
listCapsules(): Promise<DesktopCapsulesPage>;
getCapsule(capsuleId: string): Promise<DesktopCapsuleSummary | null>;
getCapsuleHtml(capsuleId: string): Promise<string>;
deleteCapsule(input: DeleteCapsuleInput): Promise<void>;
```

### Main client (`src/main/gary-client.ts`)

Add payload shape tolerant of snake_case and camelCase:

- `CapsulePayload` with `thread_id/threadId`, `html_sha256/htmlSha256`, `byte_size/byteSize`, etc.
- `CapsulesPayload` with `capsules?: CapsulePayload[]` and `capsule?: CapsulePayload | null`.
- `mapCapsuleSummary` filters rows without ids and normalizes numeric fields with `asFiniteNumber`, defaulting to safe display values.

Add `requestText(settings, path, init?)` beside `requestJson`:

- Build URL via `buildUrl(settings, path)`.
- Apply `applyGatewayAuthHeader` and `applyGatewayCustomHeaders` exactly as JSON requests do.
- Set `Accept: text/html, text/plain;q=0.9, */*;q=0.1`.
- Read `response.text()`.
- On non-OK, attempt JSON parse for gateway error shape before falling back to plaintext/status.
- Return raw text on OK.

Add methods:

- `listCapsules(settings)` -> `GET /api/capsules`, timeout 8000ms.
- `getCapsule(settings, capsuleId)` -> `GET /api/capsules/{id}`, timeout 8000ms, returns `payload.capsule ? summary : null`.
- `getCapsuleHtml(settings, capsuleId)` -> `GET /api/capsules/{id}/serve` with `requestText`, timeout 15000ms.
- `deleteCapsule(settings, input)` -> `DELETE /api/capsules/{id}`, timeout 8000ms.

### Main/preload IPC

In `src/main/index.ts`, import the new methods and register:

- `garyx:list-capsules` -> resolve settings -> `listCapsules(settings)`.
- `garyx:get-capsule` -> resolve settings -> `getCapsule(settings, capsuleId)`.
- `garyx:get-capsule-html` -> resolve settings -> `getCapsuleHtml(settings, capsuleId)`.
- `garyx:delete-capsule` -> resolve settings -> `deleteCapsule(settings, input)`.

In `src/preload/index.ts`, expose matching `GaryxDesktopApi` methods. Renderer calls only these methods; no renderer fetch to gateway.

## Renderer navigation changes

- `app-shell/types.ts`: add `'capsules'` to `ContentView`.
- `app-shell/desktop-route.ts`:
  - Include `capsules: 'capsules'` in `SIMPLE_VIEW_SEGMENTS` so `#/capsules` parses and builds.
  - Route stays a simple view, not a typed capsule-detail route, for v1.
- `app-shell/icons.tsx`: import lucide `Package` (or `Box` if the installed icon name differs) and export monochrome `CapsulesIcon` using `SettingsRailIcon`.
- `app-shell/components/AppLeftRail.tsx`:
  - Add props `isCapsulesView` and `onOpenCapsules`.
  - Add a sidebar button after Automation/Dreams and before Tasks labeled `Capsules` with `CapsulesIcon`.
  - Include Capsules in `isThreadView` exclusion.
- `app-shell/AppShell.tsx`:
  - Lazy import `CapsulesPanel`.
  - Add `const isCapsulesView = contentView === 'capsules'`.
  - Include Capsules in non-thread/context/header decisions (`shouldShowConversationRail`, `canEditThreadTitle`, `conversationContextText`, `conversationClassName`, static/full-page branch like Tasks/Dreams).
  - Pass `isCapsulesView`/`onOpenCapsules` to `AppLeftRail`; handler sets `contentView` to `capsules`.
  - Render `<CapsulesPanel agents={desktopAgents} onToast={pushToast} />` in the main branch before Tasks.
  - Keep saved/deep-link view behavior within the existing route helpers; no gateway state persistence changes.

## `CapsulesPanel.tsx` component design

New file: `src/renderer/src/app-shell/components/CapsulesPanel.tsx`.

### Props

```ts
type CapsulesPanelProps = {
  agents: DesktopCustomAgent[];
  onToast?: (message: string, tone?: ToastTone) => void;
};
```

### State machine

- `page: DesktopCapsulesPage | null` metadata list.
- `loading/error` for list refresh.
- `selectedCapsuleId` defaults to first row after load; preserved if it still exists.
- `htmlByKey: Record<string, string>` keyed as `${id}:${revision}:${htmlSha256}` to avoid stale replay after updates.
- `htmlLoadingId/htmlErrorById` for runner load.
- `deletingId` for destructive action.

Loading flow:

1. `loadCapsules()` calls `window.garyxDesktop.listCapsules()`.
2. After page load, preserve existing selection if possible, otherwise select first row.
3. When `selectedCapsule` changes, load HTML by cache key if missing via `getCapsuleHtml(id)`.
4. Refresh button re-runs metadata list; `Open`/selection reuses cached HTML only when revision+sha match.
5. Delete asks `window.confirm`, calls `deleteCapsule({ capsuleId })`, evicts cached HTML for that id, refreshes list, and shows a toast.

Race handling:

- Use a monotonically increasing request id or cancellation boolean per effect so stale HTML/list responses cannot overwrite the latest selection.

### Layout

Full-height two-column panel:

- Header: title `Capsules`, count chip, subtitle explaining "Self-contained HTML created by agents", Refresh action.
- Body: left list and right detail/runner.
- Left list rows show:
  - title or `Untitled Capsule`
  - description (clamped)
  - relative updated time (`now`, `Xm`, `Xh`, `Xd`, date fallback)
  - byte size formatted as B/KB/MB
  - creator badge: agent display/avatar if agent id resolves; otherwise fallback to agent id/provider text. Avatar uses `AgentOptionAvatar` with `providerType`/`providerIcon` from the existing `DesktopCustomAgent` object when available. No local provider switch table.
- Right detail:
  - Empty state when no capsules.
  - Loading/error state for HTML.
  - Header with title, description, metadata (revision, bytes, updated, id), `Copy ID`, `Refresh HTML`, `Delete`.
  - Runner shell containing exactly `<iframe sandbox="allow-scripts" srcDoc={html} title={...} />`.

### Security notes in code

- The component will include a short comment above the iframe noting why `allow-same-origin` and direct `/serve` URL are intentionally absent.
- No `dangerouslySetInnerHTML`, no `webview`, no `src` for gateway `/serve`.

## Styling plan (`src/renderer/src/styles.css`)

Add a scoped `capsules-*` block near `dreams/tasks` styles:

- `.conversation.capsules-view` and `.conversation.capsules-view .conversation-body` match the full-page zero-padding pattern used by tasks/workflow.
- `.capsules-page` mirrors `tasks-page`/`dreams-page` full-height layout.
- `.capsules-layout` two-column grid, responsive collapse below ~900px.
- `.capsules-list`, `.capsules-list-row`, selected/hover states use monochrome/neutral tokens, not semantic green.
- `.capsules-runner-frame` fills right pane with border/radius/background and has no pointer-host overlays.
- Reuse existing `.tasks-primary-button`, `.tasks-secondary-button`, `.tasks-icon-button`, `.tasks-state`, `.tasks-empty-state`, and `.tasks-status-chip` where appropriate.

## File diff plan

Expected desktop-only files:

- `docs/design/capsule-phaseB-impl.md` (this document)
- `desktop/garyx-desktop/src/shared/contracts.ts`
- `desktop/garyx-desktop/src/main/gary-client.ts`
- `desktop/garyx-desktop/src/main/index.ts`
- `desktop/garyx-desktop/src/preload/index.ts`
- `desktop/garyx-desktop/src/renderer/src/app-shell/types.ts`
- `desktop/garyx-desktop/src/renderer/src/app-shell/desktop-route.ts`
- `desktop/garyx-desktop/src/renderer/src/app-shell/icons.tsx`
- `desktop/garyx-desktop/src/renderer/src/app-shell/components/AppLeftRail.tsx`
- `desktop/garyx-desktop/src/renderer/src/app-shell/AppShell.tsx`
- `desktop/garyx-desktop/src/renderer/src/app-shell/components/CapsulesPanel.tsx` (new)
- `desktop/garyx-desktop/src/renderer/src/styles.css`

No gateway backend, iOS, DB, or MCP changes in this phase.

## Validation plan

Before implementation:

- Design review task to Claude with this doc path; do not write functional code until PASS.

After implementation:

1. Static/build:
   - `cd desktop/garyx-desktop && npm run build:ui`
2. Smoke:
   - `cd desktop/garyx-desktop && npm run test:smoke`
3. Packaged app proof (required because renderer/preload/IPC changed):
   - Ensure a test capsule exists via existing Phase A gateway capability. Preferred path:
     - Use `garyx mcp`/CLI if available to create a synthetic capsule through `capsule_create`, or directly seed through an authenticated local HTTP/MCP route if the CLI exposes it.
     - Fallback for UI proof only: use the running gateway's own `capsule_create` MCP tool from a real Garyx thread; avoid committing any generated test data.
   - `cd desktop/garyx-desktop && npm run dist:dir`
   - Quit stale Garyx (`osascript -e 'quit app "Garyx"'` or equivalent), open installed app (`open -a Garyx`).
   - Attach CDP: `playwright-cli -s=<session> attach --cdp=http://127.0.0.1:39222`.
   - Verify Capsules nav appears after Automation/Dreams and before Tasks.
   - Open Capsules tab, verify list loads metadata, select one capsule, and verify the runner iframe displays the synthetic HTML.
   - Inspect DOM/attributes through CDP: iframe has `sandbox="allow-scripts"`, has `srcdoc`, and does not have `allow-same-origin` or `src`.

Review gate:

- Create Claude code review task after validation with `--notify current-thread` and current worktree.
- Reviewer must actually build and verify critical path. Fix any findings and repeat review until PASS/SHIP.

Merge/handoff:

- Stage only this task's files.
- Commit using repo-configured git identity.
- Merge to `main` only after code review PASS and validation.
- Do not update parent `#TASK-1413` status manually; stop normally after summary.

## Public repository hygiene

- No real names, emails, tokens, user IDs, absolute `/Users/<real>` sample paths, or real gateway data in docs/fixtures/commit messages.
- Any sample capsule used during runtime validation is local app data only and not committed; sample text must be synthetic.
