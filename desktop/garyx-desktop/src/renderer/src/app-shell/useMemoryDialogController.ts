import { useCallback, useEffect, useRef, useState } from "react";

import type {
  DesktopAutomationSummary,
  DesktopCustomAgent,
  DesktopMemoryDocument,
} from "@shared/contracts";

export type MemoryDialogTarget =
  | {
      scope: "agent";
      agentId: string;
      title: string;
    }
  | {
      scope: "automation";
      automationId: string;
      title: string;
    };

function memoryKey(value: string, fallback: string): string {
  const trimmed = value.trim();
  const base = trimmed || fallback;
  let sanitized = "";
  for (const ch of base) {
    if (/^[a-z0-9]$/i.test(ch)) {
      sanitized += ch.toLowerCase();
    } else if (!sanitized.endsWith("-")) {
      sanitized += "-";
    }
  }
  const normalized = sanitized.replace(/^-+|-+$/g, "");
  return normalized || fallback;
}

function normalizeLocalPathForMatch(value: string): string {
  return value.replace(/\\/g, "/").replace(/\/+/g, "/");
}

export function resolveMemoryDialogTargetFromPath(
  absolutePath: string,
  automations: DesktopAutomationSummary[],
  agents: DesktopCustomAgent[],
): MemoryDialogTarget | null {
  const normalizedPath = normalizeLocalPathForMatch(absolutePath);

  const matchedAgent = agents.find((agent) => {
    return normalizedPath.endsWith(
      `/.garyx/agents/${memoryKey(agent.agentId, "agent")}/memory.md`,
    );
  });
  if (matchedAgent) {
    return {
      scope: "agent",
      agentId: matchedAgent.agentId,
      title: `${matchedAgent.displayName || matchedAgent.agentId} memory.md`,
    };
  }

  const matchedAutomation = automations.find((automation) => {
    return normalizedPath.endsWith(
      `/.garyx/automations/${memoryKey(automation.id, "automation")}/memory.md`,
    );
  });
  if (matchedAutomation) {
    return {
      scope: "automation",
      automationId: matchedAutomation.id,
      title: `${matchedAutomation.label} memory.md`,
    };
  }

  return null;
}

export function useMemoryDialogController() {
  const memoryDialogRequestIdRef = useRef(0);
  const [memoryDialogTarget, setMemoryDialogTarget] =
    useState<MemoryDialogTarget | null>(null);
  // The browser tab content is an Electron `WebContentsView` — an
  // OS-level layer that sits above every renderer-DOM modal regardless
  // of CSS z-index. Pause it while the Memory dialog is open so the
  // dialog isn't covered; bounds stay set, so unpausing re-mounts at
  // the same rect without BrowserPage having to re-sync.
  useEffect(() => {
    const open = Boolean(memoryDialogTarget);
    void window.garyxDesktop.setBrowserOverlayPaused(open);
    return () => {
      if (open) {
        void window.garyxDesktop.setBrowserOverlayPaused(false);
      }
    };
  }, [memoryDialogTarget]);
  const [memoryDialogDocument, setMemoryDialogDocument] =
    useState<DesktopMemoryDocument | null>(null);
  const [memoryDialogDraft, setMemoryDialogDraft] = useState("");
  const [memoryDialogSavedContent, setMemoryDialogSavedContent] = useState("");
  const [memoryDialogLoading, setMemoryDialogLoading] = useState(false);
  const [memoryDialogSaving, setMemoryDialogSaving] = useState(false);
  const [memoryDialogError, setMemoryDialogError] = useState<string | null>(
    null,
  );
  const [memoryDialogStatus, setMemoryDialogStatus] = useState<string | null>(
    null,
  );
  const memoryDialogDirty = memoryDialogDraft !== memoryDialogSavedContent;

  function memoryDialogInput(target: MemoryDialogTarget) {
    if (target.scope === "agent") {
      return {
        scope: "agent" as const,
        agentId: target.agentId,
      };
    }
    if (target.scope === "automation") {
      return {
        scope: "automation" as const,
        automationId: target.automationId,
      };
    }
    const exhaustive: never = target;
    return exhaustive;
  }

  const confirmDiscardMemoryChanges = useCallback((): boolean => {
    if (!memoryDialogDirty) {
      return true;
    }
    return window.confirm("Discard unsaved memory changes?");
  }, [memoryDialogDirty]);

  const openMemoryDialog = useCallback(async (target: MemoryDialogTarget) => {
    if (memoryDialogTarget && !confirmDiscardMemoryChanges()) {
      return;
    }

    const requestId = memoryDialogRequestIdRef.current + 1;
    memoryDialogRequestIdRef.current = requestId;
    setMemoryDialogTarget(target);
    setMemoryDialogDocument(null);
    setMemoryDialogDraft("");
    setMemoryDialogSavedContent("");
    setMemoryDialogLoading(true);
    setMemoryDialogSaving(false);
    setMemoryDialogError(null);
    setMemoryDialogStatus(null);

    try {
      const document = await window.garyxDesktop.readMemoryDocument(
        memoryDialogInput(target),
      );
      if (memoryDialogRequestIdRef.current !== requestId) {
        return;
      }
      setMemoryDialogDocument(document);
      setMemoryDialogDraft(document.content);
      setMemoryDialogSavedContent(document.content);
    } catch (memoryError) {
      if (memoryDialogRequestIdRef.current !== requestId) {
        return;
      }
      setMemoryDialogError(
        memoryError instanceof Error
          ? memoryError.message
          : "Failed to load memory.md.",
      );
    } finally {
      if (memoryDialogRequestIdRef.current === requestId) {
        setMemoryDialogLoading(false);
      }
    }
  }, [confirmDiscardMemoryChanges, memoryDialogTarget]);

  function closeMemoryDialog() {
    if (!confirmDiscardMemoryChanges()) {
      return;
    }

    memoryDialogRequestIdRef.current += 1;
    setMemoryDialogTarget(null);
    setMemoryDialogDocument(null);
    setMemoryDialogDraft("");
    setMemoryDialogSavedContent("");
    setMemoryDialogLoading(false);
    setMemoryDialogSaving(false);
    setMemoryDialogError(null);
    setMemoryDialogStatus(null);
  }

  async function saveMemoryDialog() {
    if (!memoryDialogTarget) {
      return;
    }

    const requestId = memoryDialogRequestIdRef.current + 1;
    memoryDialogRequestIdRef.current = requestId;
    setMemoryDialogSaving(true);
    setMemoryDialogError(null);
    setMemoryDialogStatus(null);

    try {
      const document = await window.garyxDesktop.saveMemoryDocument({
        ...memoryDialogInput(memoryDialogTarget),
        content: memoryDialogDraft,
      });
      if (memoryDialogRequestIdRef.current !== requestId) {
        return;
      }
      setMemoryDialogDocument(document);
      setMemoryDialogDraft(document.content);
      setMemoryDialogSavedContent(document.content);
      setMemoryDialogStatus("Saved memory.md.");
    } catch (memoryError) {
      if (memoryDialogRequestIdRef.current !== requestId) {
        return;
      }
      setMemoryDialogError(
        memoryError instanceof Error
          ? memoryError.message
          : "Failed to save memory.md.",
      );
    } finally {
      if (memoryDialogRequestIdRef.current === requestId) {
        setMemoryDialogSaving(false);
      }
    }
  }

  return {
    closeMemoryDialog,
    memoryDialogDirty,
    memoryDialogDocument,
    memoryDialogDraft,
    memoryDialogError,
    memoryDialogLoading,
    memoryDialogSaving,
    memoryDialogStatus,
    memoryDialogTarget,
    openMemoryDialog,
    saveMemoryDialog,
    setMemoryDialogDraft,
  };
}
