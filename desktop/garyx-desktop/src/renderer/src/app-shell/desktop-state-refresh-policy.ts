export type DesktopStateRefreshTrigger =
  | {
      kind: "connection";
      nextOk: boolean | null;
      previousOk: boolean | null;
    }
  | { kind: "mutation" }
  | { hidden: boolean; kind: "periodic" }
  | {
      hasSelectedThread: boolean;
      hidden: boolean;
      kind: "visibility";
    };

export type DesktopStateRefreshDecision = {
  desktopRefresh: "none" | "immediate" | "debounced";
  refreshSelectedThreadHistory: boolean;
  requiresVisible: boolean;
};

const NO_REFRESH: DesktopStateRefreshDecision = {
  desktopRefresh: "none",
  refreshSelectedThreadHistory: false,
  requiresVisible: false,
};

/**
 * Pure policy for root desktop-state refresh triggers. Transport coalescing
 * belongs to GatewayMirror; this function owns only event/visibility intent.
 */
export function desktopStateRefreshDecision(
  trigger: DesktopStateRefreshTrigger,
): DesktopStateRefreshDecision {
  switch (trigger.kind) {
    case "connection":
      return trigger.previousOk === false && trigger.nextOk === true
        ? {
            desktopRefresh: "immediate",
            refreshSelectedThreadHistory: false,
            requiresVisible: false,
          }
        : NO_REFRESH;
    case "mutation":
      return {
        desktopRefresh: "debounced",
        refreshSelectedThreadHistory: false,
        requiresVisible: false,
      };
    case "periodic":
      return trigger.hidden
        ? NO_REFRESH
        : {
            desktopRefresh: "debounced",
            refreshSelectedThreadHistory: false,
            requiresVisible: true,
          };
    case "visibility":
      return trigger.hidden
        ? NO_REFRESH
        : {
            desktopRefresh: "debounced",
            refreshSelectedThreadHistory: trigger.hasSelectedThread,
            requiresVisible: true,
          };
  }
}
