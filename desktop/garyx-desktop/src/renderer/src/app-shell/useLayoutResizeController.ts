import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";

import {
  DEFAULT_DESKTOP_SETTINGS,
  type DesktopSettings,
  type DesktopState,
  type WindowLayoutBootstrap,
} from "@shared/contracts";

import type { SideCapsuleTab } from "./components/SideToolsPanel";
import {
  SIDE_TOOLS_PANEL_MAX_WIDTH,
  SIDE_TOOLS_PANEL_MIN_WIDTH,
  THREAD_LOG_PANEL_MAX_WIDTH,
  THREAD_LOG_PANEL_MIN_WIDTH,
  clampSideToolsPanelWidth,
  clampThreadLogsPanelWidth,
  defaultSideToolsPanelWidth,
} from "./diagnostics-helpers";
import {
  createHorizontalLayoutFrameStore,
  createLegacyHorizontalLayoutFrameStore,
  type HorizontalLayoutFrameStore,
} from "./horizontal-layout-frame-store";
import {
  createHorizontalLayoutEffectRunner,
  type HorizontalLayoutEffectRunner,
} from "./horizontal-layout-effect-runner";
import type { LayoutOccupancyEvent } from "./layout-occupancy-events";
import {
  CONVERSATION_RAIL_DEFAULT_WIDTH,
  SIDEBAR_DEFAULT_WIDTH,
  type HorizontalLayoutEvent,
  type LayoutPanelId,
  type LayoutMachineEffect,
  type WindowLayoutSnapshot,
} from "./responsive-layout-model";
import type { ContentView } from "./types";

type UseLayoutResizeControllerArgs = {
  contentView: ContentView;
  desktopState: DesktopState | null;
  inspectorOpen: boolean;
  openCapsuleTabs: SideCapsuleTab[];
  secondaryRailOpen: boolean;
  setDesktopState: React.Dispatch<React.SetStateAction<DesktopState | null>>;
  setSettingsDraft: React.Dispatch<React.SetStateAction<DesktopSettings>>;
  threadLogsOpen: boolean;
  windowLayoutBootstrap: Readonly<{
    bootstrap: WindowLayoutBootstrap;
    rendererEpoch: string;
  }> | null;
};

function readSidebarDesiredOpen(): boolean {
  try {
    return window.localStorage.getItem("garyx.sidebarCollapsed") !== "1";
  } catch {
    return true;
  }
}

function rendererWindowSnapshot(
  revision: number,
  origin: WindowLayoutSnapshot["origin"],
): WindowLayoutSnapshot {
  const width = window.innerWidth;
  const height = window.innerHeight;
  const bounds = { x: 0, y: 0, width, height };
  return {
    windowRevision: revision,
    bounds,
    contentBounds: { ...bounds },
    normalBounds: { ...bounds },
    workArea: {
      x: 0,
      y: 0,
      width: Math.max(width, window.screen.availWidth || width),
      height: Math.max(height, window.screen.availHeight || height),
    },
    mode: "normal",
    displayId: "renderer-local",
    scaleFactor: window.devicePixelRatio,
    origin,
  };
}

export function useLayoutResizeController({
  contentView,
  desktopState,
  inspectorOpen,
  openCapsuleTabs,
  secondaryRailOpen,
  setDesktopState,
  setSettingsDraft,
  threadLogsOpen,
  windowLayoutBootstrap,
}: UseLayoutResizeControllerArgs) {
  const initialSidebarDesiredOpenRef = useRef<boolean | null>(null);
  if (initialSidebarDesiredOpenRef.current === null) {
    initialSidebarDesiredOpenRef.current = readSidebarDesiredOpen();
  }
  const layoutPolicy = window.garyxDesktop.horizontalLayoutPolicy;
  const pendingEffectsRef = useRef<LayoutMachineEffect[]>([]);
  const effectRunnerRef = useRef<HorizontalLayoutEffectRunner | null>(null);
  const storeRef = useRef<HorizontalLayoutFrameStore | null>(null);
  if (!storeRef.current) {
    const desiredOccupancy = {
      globalSidebar:
        windowLayoutBootstrap &&
        !windowLayoutBootstrap.bootstrap.freshSession
          ? windowLayoutBootstrap.bootstrap.acknowledgedSession
              .desiredOccupancy.globalSidebar
          : initialSidebarDesiredOpenRef.current,
      conversationRail: secondaryRailOpen,
      sideTools: inspectorOpen || openCapsuleTabs.length > 0,
      threadLogs: threadLogsOpen,
    };
    const widths = {
      globalSidebar: SIDEBAR_DEFAULT_WIDTH,
      conversationRail: CONVERSATION_RAIL_DEFAULT_WIDTH,
      sideTools: defaultSideToolsPanelWidth(null),
      threadLogs: DEFAULT_DESKTOP_SETTINGS.threadLogsPanelWidth,
    };
    if (layoutPolicy === "legacy") {
      storeRef.current = createLegacyHorizontalLayoutFrameStore({
        rendererEpoch: "legacy-live-renderer",
        snapshot: rendererWindowSnapshot(1, "hydrate"),
        desiredOccupancy,
        widths,
      });
    } else {
      if (!windowLayoutBootstrap) {
        throw new Error("expand-v1 requires a window layout bootstrap");
      }
      const { bootstrap, rendererEpoch } = windowLayoutBootstrap;
      const store = createHorizontalLayoutFrameStore({
        policy: "expand-v1",
        rendererEpoch,
        snapshot: bootstrap.snapshot,
        desiredOccupancy,
        widths,
        hydrated: false,
      });
      pendingEffectsRef.current.push(
        ...store.dispatch({
          type: "HYDRATE",
          freshSession: bootstrap.freshSession,
          snapshot: bootstrap.snapshot,
          desiredOccupancy,
          acknowledgedSession: bootstrap.freshSession
            ? undefined
            : bootstrap.acknowledgedSession,
        }),
      );
      storeRef.current = store;
    }
  }
  const store = storeRef.current;
  const frame = useSyncExternalStore(
    store.subscribe,
    store.getSnapshot,
    store.getSnapshot,
  );
  const layoutRootRef = useCallback(
    (root: HTMLDivElement | null) => {
      store.attachRoot(root);
    },
    [store],
  );
  useEffect(
    () => () => {
      store.attachRoot(null);
    },
    [store],
  );

  const dispatchStoreEvent = useCallback(
    (event: HorizontalLayoutEvent) => {
      if (effectRunnerRef.current) {
        effectRunnerRef.current.dispatch(event);
        return;
      }
      pendingEffectsRef.current.push(...store.dispatch(event));
    },
    [store],
  );

  useLayoutEffect(() => {
    if (layoutPolicy !== "expand-v1") {
      return;
    }
    const runner = createHorizontalLayoutEffectRunner({
      api: window.garyxDesktop,
      store,
    });
    effectRunnerRef.current = runner;
    const pending = pendingEffectsRef.current;
    pendingEffectsRef.current = [];
    runner.run(pending);
    return () => {
      if (effectRunnerRef.current === runner) {
        effectRunnerRef.current = null;
      }
      runner.stop();
    };
  }, [layoutPolicy, store]);

  const dispatchLayoutOccupancyEvent = useCallback(
    (event: LayoutOccupancyEvent) => {
      const previousFrame = store.getSnapshot();
      dispatchStoreEvent(event);
      const nextFrame = store.getSnapshot();
      if (
        nextFrame.presentation.compactViewport &&
        !previousFrame.presentation.compactViewport &&
        store.getState().compactSidebarOpen
      ) {
        dispatchStoreEvent({ type: "COMPACT_SIDEBAR_TOGGLED" });
      }
    },
    [dispatchStoreEvent, store],
  );
  const dispatchPanelWidth = useCallback(
    (panel: LayoutPanelId, width: number, commit = false) => {
      dispatchStoreEvent({
        type: "PANEL_WIDTH_CHANGED",
        panel,
        width,
        commit,
      });
    },
    [dispatchStoreEvent],
  );

  const currentConversationWidth = useCallback(() => {
    const columns = store.getSnapshot().nestedColumns.conversation;
    return columns.threadLayout + columns.sideToolsResizer + columns.sideTools;
  }, [store]);
  const currentThreadLayoutWidth = useCallback(
    () => store.getSnapshot().nestedColumns.conversation.threadLayout,
    [store],
  );

  const [sidebarResizing, setSidebarResizing] = useState(false);
  const [railResizing, setRailResizing] = useState(false);
  const [threadLogsResizing, setThreadLogsResizing] = useState(false);
  const [sideToolsResizing, setSideToolsResizing] = useState(false);
  const sidebarResizeStateRef = useRef<{
    startX: number;
    startWidth: number;
  } | null>(null);
  const railResizeStateRef = useRef<{
    startX: number;
    startWidth: number;
  } | null>(null);
  const threadLogsResizeStateRef = useRef<{
    startX: number;
    startWidth: number;
  } | null>(null);
  const sideToolsResizeStateRef = useRef<{
    startX: number;
    startWidth: number;
  } | null>(null);
  const threadLayoutRef = useRef<HTMLDivElement | null>(null);

  const persistThreadLogsPanelWidth = useCallback(
    async (nextWidth: number) => {
      const clampedWidth = clampThreadLogsPanelWidth(
        nextWidth,
        currentThreadLayoutWidth(),
      );
      dispatchPanelWidth("threadLogs", clampedWidth, true);
      setSettingsDraft((current) => ({
        ...current,
        threadLogsPanelWidth: clampedWidth,
      }));

      const persistedWidth = desktopState?.settings.threadLogsPanelWidth;
      if (persistedWidth === clampedWidth) {
        return;
      }

      try {
        const nextState = await window.garyxDesktop.saveSettings({
          ...(desktopState?.settings || DEFAULT_DESKTOP_SETTINGS),
          threadLogsPanelWidth: clampedWidth,
        });
        setDesktopState(nextState);
      } catch {
        // Keep the local width even if persistence fails; this preference is
        // deliberately non-blocking.
      }
    },
    [
      currentThreadLayoutWidth,
      desktopState?.settings,
      dispatchPanelWidth,
      setDesktopState,
      setSettingsDraft,
    ],
  );

  useEffect(() => {
    const nextWidth = clampThreadLogsPanelWidth(
      desktopState?.settings.threadLogsPanelWidth ??
        DEFAULT_DESKTOP_SETTINGS.threadLogsPanelWidth,
      currentThreadLayoutWidth(),
    );
    dispatchPanelWidth("threadLogs", nextWidth);
    setSettingsDraft((current) =>
      current.threadLogsPanelWidth === nextWidth
        ? current
        : { ...current, threadLogsPanelWidth: nextWidth },
    );
  }, [
    currentThreadLayoutWidth,
    desktopState?.settings.threadLogsPanelWidth,
    dispatchPanelWidth,
    setSettingsDraft,
  ]);

  useLayoutEffect(() => {
    const syncViewport = () => {
      const previousFrame = store.getSnapshot();
      const state = store.getState();
      if (layoutPolicy === "legacy") {
        dispatchStoreEvent({
          type: "WINDOW_SNAPSHOT_CHANGED",
          snapshot: rendererWindowSnapshot(
            state.snapshot.windowRevision + 1,
            "user",
          ),
        });
      }
      const nextFrame = store.getSnapshot();
      if (
        nextFrame.presentation.compactViewport &&
        !previousFrame.presentation.compactViewport &&
        store.getState().compactSidebarOpen
      ) {
        dispatchStoreEvent({ type: "COMPACT_SIDEBAR_TOGGLED" });
      }

      const widths = store.getState().widths;
      const nextLogsWidth = clampThreadLogsPanelWidth(
        widths.threadLogs,
        currentThreadLayoutWidth(),
      );
      if (nextLogsWidth !== widths.threadLogs) {
        dispatchPanelWidth("threadLogs", nextLogsWidth);
        setSettingsDraft((current) => ({
          ...current,
          threadLogsPanelWidth: nextLogsWidth,
        }));
      }
      const nextSideToolsWidth = widths.sideToolsCustomized
        ? clampSideToolsPanelWidth(
            widths.sideTools,
            currentConversationWidth(),
          )
        : defaultSideToolsPanelWidth(currentConversationWidth());
      if (nextSideToolsWidth !== store.getState().widths.sideTools) {
        dispatchPanelWidth("sideTools", nextSideToolsWidth);
      }
    };

    syncViewport();
    window.addEventListener("resize", syncViewport);
    return () => {
      window.removeEventListener("resize", syncViewport);
    };
  }, [
    currentConversationWidth,
    currentThreadLayoutWidth,
    dispatchStoreEvent,
    dispatchPanelWidth,
    layoutPolicy,
    setSettingsDraft,
    store,
  ]);

  useLayoutEffect(() => {
    if (
      contentView !== "thread" ||
      !(inspectorOpen || openCapsuleTabs.length > 0)
    ) {
      return;
    }
    const animationFrame = window.requestAnimationFrame(() => {
      const widths = store.getState().widths;
      const nextWidth = widths.sideToolsCustomized
        ? clampSideToolsPanelWidth(
            widths.sideTools,
            currentConversationWidth(),
          )
        : defaultSideToolsPanelWidth(currentConversationWidth());
      if (nextWidth !== widths.sideTools) {
        dispatchPanelWidth("sideTools", nextWidth);
      }
    });
    return () => {
      window.cancelAnimationFrame(animationFrame);
    };
  }, [
    contentView,
    currentConversationWidth,
    dispatchPanelWidth,
    inspectorOpen,
    openCapsuleTabs.length,
    store,
  ]);

  const toggleSidebarCollapsed = useCallback(() => {
    if (store.getSnapshot().presentation.compactViewport) {
      dispatchStoreEvent({ type: "COMPACT_SIDEBAR_TOGGLED" });
      return;
    }
    try {
      window.localStorage.setItem(
        "garyx.sidebarCollapsed",
        store.getState().desiredOccupancy.globalSidebar ? "0" : "1",
      );
    } catch {
      // Ignore storage failures; collapse state just will not persist.
    }
  }, [dispatchStoreEvent, store]);

  function handleSidebarResizeStart(
    event: React.PointerEvent<HTMLDivElement>,
  ) {
    sidebarResizeStateRef.current = {
      startX: event.clientX,
      startWidth: store.getState().widths.globalSidebar,
    };
    setSidebarResizing(true);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    event.preventDefault();
  }

  function handleRailResizeStart(event: React.PointerEvent<HTMLDivElement>) {
    railResizeStateRef.current = {
      startX: event.clientX,
      startWidth: store.getState().widths.conversationRail,
    };
    setRailResizing(true);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    event.preventDefault();
  }

  function handleThreadLogsResizeStart(
    event: React.PointerEvent<HTMLDivElement>,
  ) {
    if (!threadLogsOpen) {
      return;
    }
    threadLogsResizeStateRef.current = {
      startX: event.clientX,
      startWidth: store.getState().widths.threadLogs,
    };
    setThreadLogsResizing(true);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    event.preventDefault();
  }

  function handleThreadLogsResizeKeyDown(
    event: React.KeyboardEvent<HTMLDivElement>,
  ) {
    if (!threadLogsOpen) {
      return;
    }
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) {
      return;
    }
    event.preventDefault();
    const currentWidth = store.getState().widths.threadLogs;
    const step = event.shiftKey ? 48 : 24;
    const nextWidth =
      event.key === "Home"
        ? THREAD_LOG_PANEL_MIN_WIDTH
        : event.key === "End"
          ? clampThreadLogsPanelWidth(
              THREAD_LOG_PANEL_MAX_WIDTH,
              currentThreadLayoutWidth(),
            )
          : event.key === "ArrowLeft"
            ? clampThreadLogsPanelWidth(
                currentWidth + step,
                currentThreadLayoutWidth(),
              )
            : clampThreadLogsPanelWidth(
                currentWidth - step,
                currentThreadLayoutWidth(),
              );
    void persistThreadLogsPanelWidth(nextWidth);
  }

  function handleSideToolsResizeStart(
    event: React.PointerEvent<HTMLDivElement>,
  ) {
    const currentWidth = store.getState().widths.sideTools;
    dispatchPanelWidth("sideTools", currentWidth);
    sideToolsResizeStateRef.current = {
      startX: event.clientX,
      startWidth: currentWidth,
    };
    setSideToolsResizing(true);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    event.preventDefault();
  }

  function handleSideToolsResizeKeyDown(
    event: React.KeyboardEvent<HTMLDivElement>,
  ) {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) {
      return;
    }
    event.preventDefault();
    const currentWidth = store.getState().widths.sideTools;
    const step = event.shiftKey ? 56 : 28;
    const nextWidth =
      event.key === "Home"
        ? SIDE_TOOLS_PANEL_MIN_WIDTH
        : event.key === "End"
          ? clampSideToolsPanelWidth(
              SIDE_TOOLS_PANEL_MAX_WIDTH,
              currentConversationWidth(),
            )
          : event.key === "ArrowLeft"
            ? clampSideToolsPanelWidth(
                currentWidth + step,
                currentConversationWidth(),
              )
            : clampSideToolsPanelWidth(
                currentWidth - step,
                currentConversationWidth(),
              );
    dispatchPanelWidth("sideTools", nextWidth, true);
  }

  useEffect(() => {
    if (!sidebarResizing) {
      return;
    }
    const handlePointerMove = (event: PointerEvent) => {
      const resizeState = sidebarResizeStateRef.current;
      if (!resizeState) {
        return;
      }
      dispatchPanelWidth(
        "globalSidebar",
        resizeState.startWidth + (event.clientX - resizeState.startX),
      );
    };
    const finishResize = () => {
      sidebarResizeStateRef.current = null;
      setSidebarResizing(false);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", finishResize);
    window.addEventListener("pointercancel", finishResize);
    return () => {
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", finishResize);
      window.removeEventListener("pointercancel", finishResize);
    };
  }, [dispatchPanelWidth, sidebarResizing]);

  useEffect(() => {
    if (!railResizing) {
      return;
    }
    let lastNext =
      railResizeStateRef.current?.startWidth ??
      store.getState().widths.conversationRail;
    let animationFrame: number | null = null;
    const flush = () => {
      animationFrame = null;
      dispatchPanelWidth("conversationRail", lastNext);
    };
    const handlePointerMove = (event: PointerEvent) => {
      const resizeState = railResizeStateRef.current;
      if (!resizeState) {
        return;
      }
      lastNext =
        resizeState.startWidth + (event.clientX - resizeState.startX);
      if (animationFrame === null) {
        animationFrame = window.requestAnimationFrame(flush);
      }
    };
    const finishResize = () => {
      if (animationFrame !== null) {
        window.cancelAnimationFrame(animationFrame);
        animationFrame = null;
      }
      dispatchPanelWidth("conversationRail", lastNext, true);
      railResizeStateRef.current = null;
      setRailResizing(false);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", finishResize);
    window.addEventListener("pointercancel", finishResize);
    return () => {
      if (animationFrame !== null) {
        window.cancelAnimationFrame(animationFrame);
      }
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", finishResize);
      window.removeEventListener("pointercancel", finishResize);
    };
  }, [dispatchPanelWidth, railResizing, store]);

  useEffect(() => {
    if (!threadLogsResizing) {
      return;
    }
    const handlePointerMove = (event: PointerEvent) => {
      const resizeState = threadLogsResizeStateRef.current;
      if (!resizeState) {
        return;
      }
      const nextWidth = clampThreadLogsPanelWidth(
        resizeState.startWidth + (resizeState.startX - event.clientX),
        currentThreadLayoutWidth(),
      );
      dispatchPanelWidth("threadLogs", nextWidth);
      setSettingsDraft((current) => ({
        ...current,
        threadLogsPanelWidth: nextWidth,
      }));
    };
    const finishResize = () => {
      const nextWidth = store.getState().widths.threadLogs;
      threadLogsResizeStateRef.current = null;
      setThreadLogsResizing(false);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      void persistThreadLogsPanelWidth(nextWidth);
    };
    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", finishResize);
    window.addEventListener("pointercancel", finishResize);
    return () => {
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", finishResize);
      window.removeEventListener("pointercancel", finishResize);
    };
  }, [
    currentThreadLayoutWidth,
    dispatchPanelWidth,
    persistThreadLogsPanelWidth,
    setSettingsDraft,
    store,
    threadLogsResizing,
  ]);

  useEffect(() => {
    if (!sideToolsResizing) {
      return;
    }
    const handlePointerMove = (event: PointerEvent) => {
      const resizeState = sideToolsResizeStateRef.current;
      if (!resizeState) {
        return;
      }
      dispatchPanelWidth(
        "sideTools",
        clampSideToolsPanelWidth(
          resizeState.startWidth + (resizeState.startX - event.clientX),
          currentConversationWidth(),
        ),
      );
    };
    const finishResize = () => {
      sideToolsResizeStateRef.current = null;
      setSideToolsResizing(false);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", finishResize);
    window.addEventListener("pointercancel", finishResize);
    return () => {
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", finishResize);
      window.removeEventListener("pointercancel", finishResize);
    };
  }, [
    currentConversationWidth,
    dispatchPanelWidth,
    sideToolsResizing,
  ]);

  return {
    compactSidebarViewport: frame.presentation.compactViewport,
    currentConversationWidth,
    currentThreadLayoutWidth,
    dispatchLayoutOccupancyEvent,
    handleRailResizeStart,
    handleSidebarResizeStart,
    handleSideToolsResizeKeyDown,
    handleSideToolsResizeStart,
    handleThreadLogsResizeKeyDown,
    handleThreadLogsResizeStart,
    layoutRootRef,
    railResizing,
    sidebarCollapsed: frame.presentation.globalSidebar === "collapsed",
    sidebarDesiredOpen: store.getState().desiredOccupancy.globalSidebar,
    sidebarResizing,
    conversationRailPresented:
      frame.requestedOccupancy.conversationRail,
    sideToolsEffectiveVisible:
      frame.presentation.sideTools === "docked",
    sideToolsPresented: frame.requestedOccupancy.sideTools,
    sidebarWidth: store.getState().widths.globalSidebar,
    sideToolsPanelWidth: store.getState().widths.sideTools,
    sideToolsResizing,
    taskTreeDocked: frame.presentation.taskTreeDocked,
    threadLayoutRef,
    threadLogsDocked: frame.presentation.threadLogs === "docked",
    threadLogsPresented: frame.requestedOccupancy.threadLogs,
    threadLogsPanelWidth: store.getState().widths.threadLogs,
    threadLogsResizing,
    toggleSidebarCollapsed,
  };
}
