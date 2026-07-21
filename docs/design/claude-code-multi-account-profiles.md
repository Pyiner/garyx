# Claude Code multi-account profiles

## Goal

Garyx Mac and iOS can keep several Claude Code logins, show the active account and its Session / Weekly / scoped (including Fable) quota on the Providers page, and switch the account used by future Claude Code processes.

The system Claude Code profile remains a first-class, undeletable account. It uses Claude's ordinary default configuration (`~/.claude`) and Garyx does not inject `CLAUDE_CONFIG_DIR` for it. Managed accounts live in isolated Garyx-owned configuration directories.

Mac opens Settings at Providers. iOS keeps its existing Settings navigation and Provider entry, then adapts the same account and quota concepts to native grouped mobile management surfaces.

## Product contract

- Providers is the first Mac Settings item and the default destination when Settings opens.
- Every provider is a fully expanded native Settings section; there is no disclosure/folding state.
- Provider content uses one flat list surface with row separators. The selected account is a row, not a nested summary card.
- Surfaces, meters, selection, and status treatments stay within Garyx's black / white / neutral-gray visual language.
- Provider identity reuses the Agent avatar component and branded artwork on Mac and iOS; Claude, Codex, Antigravity, and Trae never get screen-local substitute glyphs.
- Quota is represented with horizontal linear meters.
- The Claude Code section shows the selected account, its identity/plan, default model controls, and Session / Weekly / Fable quota.
- Default model and reasoning labels consume the row's available control width and remain fully visible whenever they fit; the chips do not impose percentage caps that truncate short model names.
- The account switcher shows quota for every account so a user can compare before selecting.
- Add, rename, reauthenticate, switch, and delete are provider-level account actions.
- Account dialogs are centered in the entire desktop application window.
- The multi-step Claude login dialog keeps one `640 x 300` viewport-safe geometry across every step. Its header and footer stay anchored while only the body content changes or scrolls.
- A managed login begins with one explicit `Sign in with Claude` action. Once the Gateway returns an authorization URL, desktop opens it in the default browser once per `login_id`, immediately shows and focuses the code input, and leaves the URL visible as a clickable fallback. There is no separate Open/Open Again button.
- The system-default account can be reauthenticated but cannot be renamed or deleted.
- Deleting the active managed account switches Claude Code back to System default atomically before removing the directory.
- iOS keeps Providers at its existing Settings location. Its Claude Code row follows the selected Quota Console mobile specimen: provider identity, a tappable current-account row, and aligned Session / Weekly / Fable linear meters are visible on the first Provider screen.
- iOS account management is a native sheet, not a desktop card grid. Account rows show identity, plan, quota, monochrome selected state, and a disclosure affordance. Tapping a row opens one account detail page; switching, renaming, reauthentication, and deletion live there instead of in an ellipsis menu.
- iOS reuses its existing guided browser + paste-code login experience for System default, new managed accounts, and managed-account reauthentication. Dismissing or restarting a nonterminal login calls the cancellation endpoint so no login process or uncommitted profile remains behind.

## Configuration model

Account selection belongs to the provider, not to an agent or thread:

```yaml
provider_accounts:
  claude_code:
    active_account_id: work
    accounts:
      - id: work
        name: Work
        email: user@example.com
        organization: Example
        plan: max
        auth_method: claude.ai
        created_at: 2026-07-21T12:00:00Z
        updated_at: 2026-07-21T12:00:00Z
```

`active_account_id: null` means System default. Managed configuration paths are derived from the account ID and are never accepted from a client or serialized into thread metadata.

The managed root is adjacent to the active Garyx config file:

```text
<garyx-config-parent>/provider-accounts/claude-code/<account-id>/
```

Production therefore resolves to `~/.garyx/provider-accounts/claude-code/<account-id>/`. Each directory contains an ownership marker whose content is the account ID. Delete requires all of these checks:

1. the account exists in config;
2. the candidate has the exact managed-root/account-id shape;
3. neither managed-root component, candidate, nor marker is a symlink;
4. the canonical `provider-accounts/claude-code` chain remains directly below the canonical Garyx config parent;
5. the canonical candidate remains a direct child of the canonical managed root;
6. the marker is a regular file and contains the expected account ID.

Config changes use the Gateway's serialized `mutate_config` transaction so runtime reload and atomic persistence either both succeed or roll back.

## Gateway API

All routes use the existing Gateway authentication.

### Accounts

- `GET /api/providers/claude_code/accounts`
  - returns System default plus managed accounts, selected state, auth metadata, and per-account quota;
  - quota failures are isolated to the affected account.
- `PUT /api/providers/claude_code/accounts/active`
  - body `{ "account_id": string | null }`;
  - validates the managed account before committing selection.
- `PATCH /api/providers/claude_code/accounts/{account_id}`
  - body `{ "name": string }`;
  - managed accounts only.
- `DELETE /api/providers/claude_code/accounts/{account_id}`
  - managed accounts only;
  - removes config state first, then removes the verified owned directory. A failed directory removal is reported and logged without restoring an account that runtime has already stopped selecting.

### Login

The existing state machine remains:

- `POST /api/providers/claude_code/auth/start`
- `POST /api/providers/claude_code/auth/{login_id}/submit`
- `GET /api/providers/claude_code/auth/{login_id}`
- `DELETE /api/providers/claude_code/auth/{login_id}`

The start request gains optional account-target fields:

```json
{
  "mode": "claudeai",
  "managed_account_name": "Work",
  "account_id": null
}
```

- neither field: authenticate System default;
- `managed_account_name`: reserve a new account ID and owned directory and authenticate there;
- `account_id`: reauthenticate that existing managed account.

The response adds optional `account_id`. The auth session owns its target configuration directory and uses it for both `auth login` and `auth status`. Closing either client's login surface calls the DELETE endpoint, terminates the pending Claude process, and cleans an uncommitted managed directory after ownership validation. A newly reserved directory is also cleaned up after any terminal failure. On success, parsed auth metadata is committed; adding an account never changes the active selection, which only moves through the explicit select endpoint.

## Claude process runtime

`CLAUDE_CONFIG_DIR` is a launch-time provider setting:

- System default removes Garyx's provider-owned override and lets Claude resolve its ordinary default.
- A managed account sets `CLAUDE_CONFIG_DIR` to its derived directory.
- The bridge hot-applies provider launch environment when config reload reuses a stable provider instance, just as it already hot-applies model defaults.
- Each top-level `run_streaming` captures one immutable launch-environment snapshot. Every attempt/retry and transcript lookup for that run receives the same snapshot.
- A process that is already running is untouched. A future run, or a run restarted after process reclamation, observes the current provider selection.
- Threads continue to store provider/session metadata only. Account ID and config directory are deliberately not snapshotted.
- If a Claude session ID is absent in the newly selected config directory, the existing `session not found -> fresh session` fallback starts a new provider session. Garyx transcript state preserves the conversation context.

The Claude Agent SDK remains the execution integration; it launches the configured native Claude/cctty executable. Garyx is not automating an interactive terminal itself.

## cctty dependency contract

`cctty::auth::AuthLoginOptions` and `AuthStatusOptions` need a per-call environment overlay. Garyx passes `CLAUDE_CONFIG_DIR` through that API for managed profiles. It must not mutate the Gateway process environment because concurrent logins and runs may target different accounts.

The cctty change is covered by fake-Claude tests proving the same overlay reaches both `auth login` and `auth status`, then Garyx updates its pinned cctty revision.

## Quota and credentials

Claude quota fetch accepts an optional configuration directory:

- System default keeps the legacy macOS Keychain service `Claude Code-credentials` and `~/.claude/.credentials.json` fallback.
- Managed profiles read `<config-dir>/.credentials.json` and, on macOS, Claude's scoped Keychain service `Claude Code-credentials-<sha256(config-dir)[0..8]>`.
- Usage cache keys include the account identity/config directory, so one account cannot overwrite another's cache.
- `/api/usage/coding` reports the active account for backward compatibility with existing desktop/iOS consumers.
- the account-list endpoint fetches account quotas concurrently and returns Session, Weekly, and all scoped limits, including Fable.

## Desktop structure

The Providers page replaces the separate quota hero plus configuration table with one native Settings section per provider. Each section has a single flat list surface with separated rows. Claude Code owns its account selector, quota rows, and actions; the other providers keep their existing model configuration behavior in the same Settings grammar. There are no nested account cards, colored quota surfaces, or dashboard-style hero treatments.

Desktop calls the account/auth APIs through typed main-process IPC methods. External authorization URLs are validated as HTTP(S) before using Electron's external URL opener. The renderer tracks opened `login_id` values in component state so React rerenders and polling cannot open duplicate browser tabs.

## iOS structure

iOS keeps the existing Settings route and Provider destination. The Provider screen removes the duplicate quota hero and gives Claude Code the selected mobile Quota Console treatment inside the existing grouped surface:

1. provider identity and edit affordance;
2. one current-account row that opens account management;
3. Session, Weekly, and scoped limits such as Fable as aligned horizontal meters;
4. the full default-model label in the footer row.

Other providers retain their existing native compact rows and inline quota treatment. The Claude account manager uses a native sheet with a navigation toolbar. Each candidate renders its own quota so selection is informed; tapping it opens a native account detail page. That page offers `Use This Account` when needed, plus reauthentication, rename, and confirmed deletion for managed accounts. System default offers selection and reauthentication only. A plus toolbar action opens the alias-first add flow. The list, first-screen console, and detail page use the same monochrome meter treatment.

The existing guided iOS login sheet accepts an explicit target (`system default`, `new managed name`, or `existing managed id`). It keeps the browser/paste-code interaction, refreshes both account-list and active-provider usage after success, and cancels a nonterminal Gateway session when dismissed or restarted. Core owns all wire models, target encoding, account presentation, and selection semantics; SwiftUI owns only composition and local focus/presentation state.

## Failure behavior

- Unknown/stale selected account: runtime quarantines launches in an isolated invalid-selection directory instead of silently using System default; the accounts endpoint returns no selected account and a subsequent explicit selection repairs config.
- Account quota unavailable: keep the account selectable and show an inline unavailable state.
- Browser opener failure: keep the URL visible and show a non-blocking error.
- Auth status metadata unavailable after successful login: keep the successful account and surface the status warning; quota/auth refresh can repair metadata later.
- Runtime config reload failure: `mutate_config` restores the previous config and selected account.
- Managed directory cleanup failure: never recurse outside the verified root; report the retained on-disk data for manual recovery.

## Verification

1. Model serialization/backward-compatibility tests (`provider_accounts` omitted => System default).
2. Managed-path ownership and malicious symlink/path deletion tests.
3. Auth API tests for system default, managed add, managed reauth, failure cleanup, and config-dir propagation.
4. Credential/keychain service and per-account cache-key unit tests.
5. Bridge tests proving selection hot reload affects the next run, not an active run, and no account value enters thread metadata.
6. Desktop tests for Providers-first navigation, flat expanded linear-meter sections, fixed login-dialog geometry, one browser open per login ID, immediate code field, account switching, and Fable rendering.
7. SwiftPM tests for account decoding, optional login-target encoding, selection presentation, Session / Weekly / Fable display order, and cancellation fencing; app-target tests for Gateway requests and model state transitions.
8. iOS simulator checks for the first-screen Quota Console row, account switcher, add/rename/delete/reauth presentation, Dynamic Type, light/dark mode, and login dismissal cleanup, plus a full `xcodebuild`.
9. Focused Rust tests, desktop typecheck/unit tests, then packaged Mac and simulator iOS end-to-end passes with screenshots.
