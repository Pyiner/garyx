import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

import {
  DEFAULT_DESKTOP_SETTINGS,
  type DesktopSettings,
  type DesktopState,
} from "@shared/contracts";

import type { SideCapsuleTab } from "./components/SideToolsPanel";
import {
  THREAD_LOG_PANEL_MAX_WIDTH,
  THREAD_LOG_PANEL_MIN_WIDTH,
  clampSideToolsPanelWidth,
  clampThreadLogsPanelWidth,
  defaultSideToolsPanelWidth,
} from "./diagnostics-helpers";
import type { ContentView } from "./types";
import {
  CONVERSATION_RAIL_DEFAULT_WIDTH,
  SIDEBAR_DEFAULT_WIDTH,
  clampConversationRailWidth,
  clampSidebarWidth,
  isCompactSidebarViewport,
  isDockedSidePanel,
  resolveSidebarCollapsed,
} from "./responsive-layout-model";

type UseLayoutResizeControllerArgs = {
  contentView: ContentView;
  desktopState: DesktopState | null;
  inspectorOpen: boolean;
  openCapsuleTabs: SideCapsuleTab[];
  secondaryRailOpen: boolean;
  setDesktopState: React.Dispatch<React.SetStateAction<DesktopState | null>>;
  setSettingsDraft: React.Dispatch<React.SetStateAction<DesktopSettings>>;
  threadLogsOpen: boolean;
};

export function useLayoutResizeController({
  contentView,
  desktopState,
  inspectorOpen,
  openCapsuleTabs,
  secondaryRailOpen,
  setDesktopState,
  setSettingsDraft,
  threadLogsOpen,
}: UseLayoutResizeControllerArgs) {
  const [threadLogsPanelWidth, setThreadLogsPanelWidth] = useState(
    DEFAULT_DESKTOP_SETTINGS.threadLogsPanelWidth,
  );
  const [threadLogsResizing, setThreadLogsResizing] = useState(false);
  const [sideToolsPanelWidth, setSideToolsPanelWidth] = useState(() =>
    defaultSideToolsPanelWidth(null),
  );
  const [sideToolsResizing, setSideToolsResizing] = useState(false);
  const [sidebarWidth, setSidebarWidth] = useState(SIDEBAR_DEFAULT_WIDTH);
  const [sidebarCollapsedByUser, setSidebarCollapsedByUser] = useState(() => {
    try {
      return window.localStorage.getItem("garyx.sidebarCollapsed") === "1";
    } catch {
      return false;
    }
  });
  const initialCompactSidebarViewport = isCompactSidebarViewport({
    secondaryRailOpen,
    viewportWidth: window.innerWidth,
  });
  const [compactSidebarViewport, setCompactSidebarViewport] = useState(
    initialCompactSidebarViewport,
  );
  const compactSidebarViewportRef = useRef(initialCompactSidebarViewport);
  const [compactSidebarOpen, setCompactSidebarOpen] = useState(false);
  const sidebarCollapsed = resolveSidebarCollapsed({
    compactOpen: compactSidebarOpen,
    compactViewport: compactSidebarViewport,
    userCollapsed: sidebarCollapsedByUser,
  });
  const toggleSidebarCollapsed = useCallback(() => {
    if (compactSidebarViewport) {
      setCompactSidebarOpen((current) => !current);
      return;
    }
    setSidebarCollapsedByUser((current) => {
      const next = !current;
      try {
        window.localStorage.setItem("garyx.sidebarCollapsed", next ? "1" : "0");
      } catch {
        // Ignore storage failures; collapse state just won't persist.
      }
      return next;
    });
  }, [compactSidebarViewport]);
  const [sidebarResizing, setSidebarResizing] = useState(false);
  const [railWidth, setRailWidth] = useState(CONVERSATION_RAIL_DEFAULT_WIDTH);
  const [railResizing, setRailResizing] = useState(false);
  const sidebarResizeStateRef = useRef<{
    startX: number;
    startWidth: number;
  } | null>(null);
  const railResizeStateRef = useRef<{
    startX: number;
    startWidth: number;
  } | null>(null);

  useLayoutEffect(() => {
    const root = document.documentElement;
    root.style.setProperty("--app-sidebar-width", `${sidebarWidth}px`);
    return () => {
      root.style.removeProperty("--app-sidebar-width");
    };
  }, [sidebarWidth]);

  useLayoutEffect(() => {
    const root = document.documentElement;
    root.style.setProperty("--spacing-token-rail", `${railWidth}px`);
    return () => {
      root.style.removeProperty("--spacing-token-rail");
    };
  }, [railWidth]);

  useEffect(() => {
    const syncCompactSidebar = () => {
      const nextCompact = isCompactSidebarViewport({
        secondaryRailOpen,
        viewportWidth: window.innerWidth,
      });
      if (nextCompact && !compactSidebarViewportRef.current) {
        setCompactSidebarOpen(false);
      }
      compactSidebarViewportRef.current = nextCompact;
      setCompactSidebarViewport(nextCompact);
    };

    syncCompactSidebar();
    window.addEventListener("resize", syncCompactSidebar);
    return () => {
      window.removeEventListener("resize", syncCompactSidebar);
    };
  }, [secondaryRailOpen]);
  const threadLayoutRef = useRef<HTMLDivElement | null>(null);
  const conversationRef = useRef<HTMLElement | null>(null);
  const [threadLayoutWidth, setThreadLayoutWidth] = useState(0);
  const threadLogsPanelWidthRef = useRef(
    DEFAULT_DESKTOP_SETTINGS.threadLogsPanelWidth,
  );
  const threadLogsResizeStateRef = useRef<{
    startX: number;
    startWidth: number;
  } | null>(null);
  const sideToolsPanelWidthRef = useRef(defaultSideToolsPanelWidth(null));
  const sideToolsResizeStateRef = useRef<{
    startX: number;
    startWidth: number;
  } | null>(null);
  const sideToolsPanelWidthCustomizedRef = useRef(false);

  const desktopStateReady = desktopState !== null;
  useLayoutEffect(() => {
    const threadLayout = threadLayoutRef.current;
    const syncMeasuredWidths = () => {
      const nextThreadLayoutWidth = threadLayout?.clientWidth || 0;
      setThreadLayoutWidth((current) =>
        current === nextThreadLayoutWidth ? current : nextThreadLayoutWidth,
      );
    };

    syncMeasuredWidths();
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", syncMeasuredWidths);
      return () => {
        window.removeEventListener("resize", syncMeasuredWidths);
      };
    }

    const observer = new ResizeObserver(syncMeasuredWidths);
    if (threadLayout) {
      observer.observe(threadLayout);
    }
    return () => observer.disconnect();
  }, [
    contentView,
    desktopStateReady,
    inspectorOpen,
    openCapsuleTabs.length,
    threadLogsOpen,
  ]);

  function currentThreadLayoutWidth(): number | null {
    return threadLayoutRef.current?.clientWidth || null;
  }

  function currentConversationWidth(): number | null {
    return conversationRef.current?.clientWidth || null;
  }

  async function persistThreadLogsPanelWidth(nextWidth: number) {
    const clampedWidth = clampThreadLogsPanelWidth(
      nextWidth,
      currentThreadLayoutWidth(),
    );
    setThreadLogsPanelWidth(clampedWidth);
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
      // Keep the local width even if persistence fails; this is a non-blocking UI preference.
    }
  }

  function handleSidebarResizeStart(event: React.PointerEvent<HTMLDivElement>) {
    sidebarResizeStateRef.current = {
      startX: event.clientX,
      startWidth: sidebarWidth,
    };
    setSidebarResizing(true);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    event.preventDefault();
  }

  function handleRailResizeStart(event: React.PointerEvent<HTMLDivElement>) {
    railResizeStateRef.current = {
      startX: event.clientX,
      startWidth: railWidth,
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
      startWidth: threadLogsPanelWidthRef.current,
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
                threadLogsPanelWidthRef.current + step,
                currentThreadLayoutWidth(),
              )
            : clampThreadLogsPanelWidth(
                threadLogsPanelWidthRef.current - step,
                currentThreadLayoutWidth(),
              );
    void persistThreadLogsPanelWidth(nextWidth);
  }

  useEffect(() => {
    threadLogsPanelWidthRef.current = threadLogsPanelWidth;
  }, [threadLogsPanelWidth]);

  useEffect(() => {
    sideToolsPanelWidthRef.current = sideToolsPanelWidth;
  }, [sideToolsPanelWidth]);

  useEffect(() => {
    const nextWidth = clampThreadLogsPanelWidth(
      desktopState?.settings.threadLogsPanelWidth ??
        DEFAULT_DESKTOP_SETTINGS.threadLogsPanelWidth,
      currentThreadLayoutWidth(),
    );
    setThreadLogsPanelWidth(nextWidth);
    setSettingsDraft((current) => {
      if (current.threadLogsPanelWidth === nextWidth) {
        return current;
      }
      return {
        ...current,
        threadLogsPanelWidth: nextWidth,
      };
    });
  }, [desktopState?.settings.threadLogsPanelWidth]);

  useLayoutEffect(() => {
    // Initialize/clamp the dock width whenever the dock is shown — including the
    // no-workspace capsule-only dock (#TASK-1470); width is workspace-agnostic.
    if (
      contentView !== "thread" ||
      !(inspectorOpen || openCapsuleTabs.length > 0)
    ) {
      return;
    }

    const frame = window.requestAnimationFrame(() => {
      const layoutWidth = currentConversationWidth();
      const nextWidth = sideToolsPanelWidthCustomizedRef.current
        ? clampSideToolsPanelWidth(sideToolsPanelWidthRef.current, layoutWidth)
        : defaultSideToolsPanelWidth(layoutWidth);
      if (nextWidth !== sideToolsPanelWidthRef.current) {
        setSideToolsPanelWidth(nextWidth);
      }
    });

    return () => {
      window.cancelAnimationFrame(frame);
    };
  }, [contentView, inspectorOpen, openCapsuleTabs.length]);

  useEffect(() => {
    const handleResize = () => {
      const measuredThreadLayoutWidth = currentThreadLayoutWidth() || 0;
      const measuredConversationWidth = currentConversationWidth() || 0;
      setThreadLayoutWidth(measuredThreadLayoutWidth);
      const nextWidth = clampThreadLogsPanelWidth(
        threadLogsPanelWidthRef.current,
        measuredThreadLayoutWidth,
      );
      if (nextWidth !== threadLogsPanelWidthRef.current) {
        setThreadLogsPanelWidth(nextWidth);
        setSettingsDraft((current) => ({
          ...current,
          threadLogsPanelWidth: nextWidth,
        }));
      }
      const nextSideToolsWidth = sideToolsPanelWidthCustomizedRef.current
        ? clampSideToolsPanelWidth(
            sideToolsPanelWidthRef.current,
            measuredConversationWidth,
          )
        : defaultSideToolsPanelWidth(measuredConversationWidth);
      if (nextSideToolsWidth !== sideToolsPanelWidthRef.current) {
        setSideToolsPanelWidth(nextSideToolsWidth);
      }
    };
    window.addEventListener("resize", handleResize);
    return () => {
      window.removeEventListener("resize", handleResize);
    };
  }, []);

  useEffect(() => {
    if (!sidebarResizing) {
      return;
    }
    const handlePointerMove = (event: PointerEvent) => {
      const state = sidebarResizeStateRef.current;
      if (!state) return;
      const next = clampSidebarWidth(
        state.startWidth + (event.clientX - state.startX),
      );
      setSidebarWidth(next);
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
  }, [sidebarResizing]);

  useEffect(() => {
    if (!railResizing) {
      return;
    }
    let lastNext = railResizeStateRef.current?.startWidth ?? railWidth;
    let rafId: number | null = null;
    const flush = () => {
      rafId = null;
      document.documentElement.style.setProperty(
        "--spacing-token-rail",
        `${lastNext}px`,
      );
    };
    const handlePointerMove = (event: PointerEvent) => {
      const state = railResizeStateRef.current;
      if (!state) return;
      lastNext = clampConversationRailWidth(
        state.startWidth + (event.clientX - state.startX),
      );
      if (rafId === null) {
        rafId = requestAnimationFrame(flush);
      }
    };
    const finishResize = () => {
      if (rafId !== null) {
        cancelAnimationFrame(rafId);
        rafId = null;
      }
      railResizeStateRef.current = null;
      setRailResizing(false);
      setRailWidth(lastNext);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", finishResize);
    window.addEventListener("pointercancel", finishResize);
    return () => {
      if (rafId !== null) {
        cancelAnimationFrame(rafId);
      }
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", finishResize);
      window.removeEventListener("pointercancel", finishResize);
    };
  }, [railResizing]);

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
      setThreadLogsPanelWidth(nextWidth);
      setSettingsDraft((current) => ({
        ...current,
        threadLogsPanelWidth: nextWidth,
      }));
    };

    const finishResize = () => {
      const nextWidth = threadLogsPanelWidthRef.current;
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
  }, [threadLogsResizing, desktopState?.settings.threadLogsPanelWidth]);

  useEffect(() => {
    if (!sideToolsResizing) {
      return;
    }

    const handlePointerMove = (event: PointerEvent) => {
      const resizeState = sideToolsResizeStateRef.current;
      if (!resizeState) {
        return;
      }
      const nextWidth = clampSideToolsPanelWidth(
        resizeState.startWidth + (resizeState.startX - event.clientX),
        currentConversationWidth(),
      );
      setSideToolsPanelWidth(nextWidth);
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
  }, [sideToolsResizing]);

  const threadLogsDocked = isDockedSidePanel({
    canvasWidth: threadLayoutWidth,
    panelWidth: threadLogsPanelWidth,
  });

  return {
    compactSidebarViewport,
    conversationRef,
    currentConversationWidth,
    currentThreadLayoutWidth,
    handleRailResizeStart,
    handleSidebarResizeStart,
    handleThreadLogsResizeKeyDown,
    handleThreadLogsResizeStart,
    railResizing,
    setSideToolsPanelWidth,
    setSideToolsResizing,
    sidebarCollapsed,
    sidebarDesiredOpen: !sidebarCollapsedByUser,
    sidebarResizing,
    sidebarWidth,
    sideToolsPanelWidth,
    sideToolsPanelWidthCustomizedRef,
    sideToolsPanelWidthRef,
    sideToolsResizeStateRef,
    sideToolsResizing,
    threadLayoutRef,
    threadLogsDocked,
    threadLogsPanelWidth,
    threadLogsResizing,
    toggleSidebarCollapsed,
  };
}
