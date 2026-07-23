export type WorkspaceMenuPointerTarget = {
  closest: (selector: string) => unknown;
};

/**
 * The workspace menu is controlled by AppShell, so a pointer outside its
 * trigger/actions region dismisses it before the following click/select.
 */
export function shouldDismissWorkspaceMenuOnPointerDown(
  target: WorkspaceMenuPointerTarget,
): boolean {
  return !(
    target.closest(".workspace-actions") ||
    target.closest("[data-workspace-menu-content]")
  );
}
