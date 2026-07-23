/**
 * Shell-owned visibility state for the thread side-tools rail.
 *
 * Visibility and built-in tab selection share this owner: restoring an open
 * source thread must restore the tool that made the rail meaningful, rather
 * than reopening an empty picker. The store is intentionally UI-free so
 * source-thread ownership can be exercised without mounting AppShell.
 *
 * Its snapshot follows the useSyncExternalStore contract: a cached reference
 * is replaced only after a real write.
 */
import type {
  SideTabKey,
  ThreadSideToolId,
} from "./components/side-tools-panel-model";

export interface ThreadSideToolsPanelState {
  readonly open: boolean;
  readonly openTools: ThreadSideToolId[];
  readonly activeTabKey: SideTabKey | null;
}

export type ThreadSideToolsPanelStateUpdate = (
  current: ThreadSideToolsPanelState,
) => ThreadSideToolsPanelState;

export interface ThreadSideToolsVisibilitySnapshot {
  readonly version: number;
  readonly gatewayScope: string;
  readonly unboundInitialPanel: ThreadSideToolsPanelState | null;
  readonly panelBySource: Readonly<
    Record<string, ThreadSideToolsPanelState>
  >;
}

const CLOSED_PANEL_STATE: ThreadSideToolsPanelState = Object.freeze({
  open: false,
  openTools: [],
  activeTabKey: null,
});

export function threadSideToolsPanelState(
  snapshot: ThreadSideToolsVisibilitySnapshot,
  sourceThreadId: string | null,
): ThreadSideToolsPanelState {
  return (
    (sourceThreadId && snapshot.panelBySource[sourceThreadId]) ||
    snapshot.unboundInitialPanel ||
    CLOSED_PANEL_STATE
  );
}

export class ThreadSideToolsVisibility {
  private gatewayScopeValue = "";
  private hasAdoptedGatewayScope = false;
  private unboundInitialPanel: ThreadSideToolsPanelState | null = null;
  private panelBySource: Record<string, ThreadSideToolsPanelState> = {};
  private version = 0;
  private snapshot: ThreadSideToolsVisibilitySnapshot | null = null;
  private listeners = new Set<() => void>();

  constructor(
    initialSourceThreadId: string | null,
    initiallyOpen: boolean,
  ) {
    if (initialSourceThreadId && initiallyOpen) {
      this.panelBySource = {
        [initialSourceThreadId]: {
          open: true,
          openTools: [],
          activeTabKey: null,
        },
      };
    } else if (initiallyOpen) {
      // A restored native layout may arrive before the thread route is
      // hydrated. Keep that occupancy visible, then bind it to exactly the
      // first committed source in adoptInitialSource().
      this.unboundInitialPanel = {
        open: true,
        openTools: [],
        activeTabKey: null,
      };
    }
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  getSnapshot = (): ThreadSideToolsVisibilitySnapshot => {
    if (!this.snapshot) {
      this.snapshot = {
        version: this.version,
        gatewayScope: this.gatewayScopeValue,
        unboundInitialPanel: this.unboundInitialPanel,
        panelBySource: this.panelBySource,
      };
    }
    return this.snapshot;
  };

  private commit(): void {
    this.version += 1;
    this.snapshot = null;
    for (const listener of this.listeners) {
      listener();
    }
  }

  /**
   * Preserve the restored initial route through first hydration, then clear
   * the domain on every real gateway transition because thread ids are only
   * unique inside one gateway.
   */
  setGatewayScope(scope: string): void {
    if (!this.hasAdoptedGatewayScope) {
      if (!scope) {
        return;
      }
      this.hasAdoptedGatewayScope = true;
      this.gatewayScopeValue = scope;
      this.commit();
      return;
    }
    if (this.gatewayScopeValue === scope) {
      return;
    }
    this.gatewayScopeValue = scope;
    this.unboundInitialPanel = null;
    this.panelBySource = {};
    this.commit();
  }

  adoptInitialSource(sourceThreadId: string | null): void {
    if (!sourceThreadId || !this.unboundInitialPanel) {
      return;
    }
    this.panelBySource = {
      ...this.panelBySource,
      [sourceThreadId]: this.unboundInitialPanel,
    };
    this.unboundInitialPanel = null;
    this.commit();
  }

  panelFor(sourceThreadId: string | null): ThreadSideToolsPanelState {
    return threadSideToolsPanelState(this.getSnapshot(), sourceThreadId);
  }

  isOpen(sourceThreadId: string | null): boolean {
    return this.panelFor(sourceThreadId).open;
  }

  setOpen(sourceThreadId: string | null, open: boolean): void {
    if (!sourceThreadId) {
      return;
    }
    const current = this.panelFor(sourceThreadId);
    if (current.open === open) {
      return;
    }
    if (!open) {
      const next = { ...this.panelBySource };
      delete next[sourceThreadId];
      this.panelBySource = next;
      this.unboundInitialPanel = null;
    } else {
      this.panelBySource = {
        ...this.panelBySource,
        [sourceThreadId]: {
          ...current,
          open: true,
        },
      };
      this.unboundInitialPanel = null;
    }
    this.commit();
  }

  updatePanel(
    sourceThreadId: string | null,
    update: ThreadSideToolsPanelStateUpdate,
  ): void {
    if (!sourceThreadId) {
      return;
    }
    const current = this.panelFor(sourceThreadId);
    const next = update(current);
    if (
      next === current ||
      (next.open === current.open &&
        next.openTools === current.openTools &&
        next.activeTabKey === current.activeTabKey)
    ) {
      return;
    }
    this.panelBySource = {
      ...this.panelBySource,
      [sourceThreadId]: next,
    };
    this.unboundInitialPanel = null;
    this.commit();
  }
}
