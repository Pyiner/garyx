# Workspace And Path Model

Workspace identity is the absolute directory path everywhere: desktop, mobile,
gateway, and CLI flows must pass and persist the path string directly.

## Rules

- Do not add workspace IDs; workspaces are directory filters/bookmarks, not a
  separate domain entity.
- Desktop and mobile root workspace lists must contain only user-added
  workspaces persisted by gateway `/api/workspaces` application SQLite state.
- Thread `workspacePath` values and temporary workspace paths are metadata only.
  They may help with sorting, file-link resolution, or form suggestions, but
  must not create inferred root workspace rows.
- If the gateway workspace table has no rows, gateway initialization may seed it
  once from configured bot accounts and scheduled automation jobs.
- Soft-deleted rows count as existing state and must prevent future inferred
  reseeding.
- Desktop and mobile workspace fields should expose a platform-local shared
  workspace select.
- Workspace options come from `/api/workspaces`; selected business values are
  always absolute path strings.
- If a current path is no longer in the saved list, keep displaying and
  submitting it unchanged until the user picks a different workspace.
- The final workspace select item should be `Add workspace`.
- `Add workspace` opens a lightweight directory browser that shows the current
  folder, immediate child folders, back navigation, and an explicit "use this
  folder" action.
- Add through the backend workspace API, refresh options, and set the form field
  to the added absolute path.
- Do not use native file-manager dialogs or raw path inputs as the primary
  control in ordinary forms.
