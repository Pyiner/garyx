// DesktopRouteStore: the URL-hash single source of truth for the desktop
// route (endgame architecture batch 4a,
// docs/design/appshell-endgame-architecture.md "Route as URL single source
// of truth").
//
// The store mirrors location.hash: `navigate` is the only hash writer, and
// external hash/popstate changes update the store first — route effects
// follow the store and never counter-write the hash. Batch 4a ships the
// store additively (nothing wired); AppShell ownership flips in 4b.
//
// Hard rule (useSyncExternalStore compatibility, same as GatewayMirror):
// getSnapshot returns a cached object reference reused until the route
// actually changes.

import {
  buildDesktopRouteHash,
  canonicalDesktopRoute,
  desktopRoutesEqual,
  parseDesktopRoute,
  type DesktopRoute,
} from "./desktop-route.ts";

export type Unsubscribe = () => void;

/**
 * The window seam the store drives, injected so the store stays pure
 * TypeScript and node-testable. `subscribe` delivers external hash
 * changes (hashchange/popstate — including echoes of our own writes,
 * which the store dedupes by route equality).
 */
export interface RouteHost {
  getHref(): string;
  /** history.replaceState: no new history entry. */
  replaceHash(hash: string): void;
  /** location.hash assignment: pushes a history entry. */
  pushHash(hash: string): void;
  subscribe(onExternalChange: () => void): Unsubscribe;
}

export function createBrowserRouteHost(): RouteHost {
  return {
    getHref: () => window.location.href,
    replaceHash: (hash) => {
      window.history.replaceState(null, "", hash);
    },
    pushHash: (hash) => {
      window.location.hash = hash;
    },
    subscribe: (onExternalChange) => {
      window.addEventListener("hashchange", onExternalChange);
      window.addEventListener("popstate", onExternalChange);
      return () => {
        window.removeEventListener("hashchange", onExternalChange);
        window.removeEventListener("popstate", onExternalChange);
      };
    },
  };
}

export interface DesktopRouteSnapshot {
  readonly version: number;
  readonly route: DesktopRoute;
}

/**
 * One route commit, delivered synchronously from inside commit() (batch
 * 6c-2a). `origin` distinguishes the commit sources so the route effect
 * can decide whether and how to apply:
 * - `navigate`: an internal user/program navigation — applied, and its
 *   failure converges the hash back to the settled state.
 * - `external`: hash/popstate application — applied; a failure never
 *   counter-writes the entered hash (4b).
 * - `sync`: a state-to-hash synchronization (`syncRoute`) — the state
 *   ALREADY IS this route, so the route effect must not re-apply it
 *   (re-applying would re-run entry side effects like the new-thread
 *   branch's clearComposerDraft against a live draft). uSES subscribers
 *   still see the commit.
 * `version` and `route` match getSnapshot() at delivery time (the
 * snapshot is committed first).
 */
export interface RouteCommitEvent {
  readonly route: DesktopRoute;
  readonly version: number;
  readonly origin: "navigate" | "external" | "sync";
}

export class DesktopRouteStore {
  private host: RouteHost;
  private route: DesktopRoute;
  private version = 0;
  private snapshot: DesktopRouteSnapshot | null = null;
  private listeners = new Set<() => void>();
  private commitListeners = new Set<(event: RouteCommitEvent) => void>();
  private unsubscribeHost: Unsubscribe;

  constructor(host: RouteHost) {
    this.host = host;
    this.route = parseDesktopRoute(host.getHref());
    this.unsubscribeHost = host.subscribe(() => {
      this.onExternalChange();
    });
  }

  subscribe(listener: () => void): Unsubscribe {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  /**
   * Notified synchronously for EVERY route commit — internal navigations
   * and external hash/popstate applications alike — with the committed
   * canonical route, its store version, and the commit origin (batch
   * 6c-2a). Delivery order per commit: plain subscribe() listeners (the
   * uSES faces) first, then commit listeners.
   */
  subscribeCommits(listener: (event: RouteCommitEvent) => void): Unsubscribe {
    this.commitListeners.add(listener);
    return () => {
      this.commitListeners.delete(listener);
    };
  }

  getSnapshot(): DesktopRouteSnapshot {
    if (!this.snapshot) {
      this.snapshot = {
        version: this.version,
        route: this.route,
      };
    }
    return this.snapshot;
  }

  /**
   * The only hash writer. An equal route with an already-canonical hash is
   * a full no-op; an equal route whose hash text differs (legacy alias
   * like #/threads/<id>) canonicalizes the hash via replace without a
   * store commit (the route did not change). A different route writes the
   * hash (push by default, replace on request) and commits.
   */
  navigate(route: DesktopRoute, options?: { replace?: boolean }): void {
    const nextHash = buildDesktopRouteHash(route);
    const currentHash = this.currentHostHash();
    if (desktopRoutesEqual(route, this.route)) {
      if (nextHash !== currentHash) {
        this.host.replaceHash(nextHash);
      }
      return;
    }
    // Commit the CANONICAL route before writing the hash: the write's own
    // hashchange echo parses to exactly the canonical form (hash builds
    // drop default/empty params, e.g. the new-thread 'claude' agent), so
    // onExternalChange's equality dedupe holds for every navigable route.
    this.commit(canonicalDesktopRoute(route), "navigate");
    if (options?.replace) {
      this.host.replaceHash(nextHash);
    } else {
      this.host.pushHash(nextHash);
    }
  }

  /**
   * State-to-hash synchronization (batch 6c-2a): commit + replace-write a
   * route the application state ALREADY reflects (the fold over settled
   * state). Identical to navigate({replace:true}) except the commit
   * carries origin 'sync', which the route effect does not re-apply.
   */
  syncRoute(route: DesktopRoute): void {
    const nextHash = buildDesktopRouteHash(route);
    if (desktopRoutesEqual(route, this.route)) {
      if (nextHash !== this.currentHostHash()) {
        this.host.replaceHash(nextHash);
      }
      return;
    }
    this.commit(canonicalDesktopRoute(route), "sync");
    this.host.replaceHash(nextHash);
  }

  dispose(): void {
    this.unsubscribeHost();
    this.listeners.clear();
    this.commitListeners.clear();
  }

  private currentHostHash(): string {
    try {
      return new URL(this.host.getHref()).hash;
    } catch {
      return "";
    }
  }

  private onExternalChange(): void {
    const parsed = parseDesktopRoute(this.host.getHref());
    // Echoes of our own navigate() (hash assignment fires hashchange) and
    // no-op rewrites parse back to the committed route: ignore.
    if (desktopRoutesEqual(parsed, this.route)) {
      return;
    }
    this.commit(parsed, "external");
  }

  private commit(
    route: DesktopRoute,
    origin: "navigate" | "external" | "sync",
  ): void {
    this.route = route;
    this.version += 1;
    this.snapshot = null;
    for (const listener of [...this.listeners]) {
      listener();
    }
    const event: RouteCommitEvent = {
      route: this.route,
      version: this.version,
      origin,
    };
    for (const listener of [...this.commitListeners]) {
      listener(event);
    }
  }
}
