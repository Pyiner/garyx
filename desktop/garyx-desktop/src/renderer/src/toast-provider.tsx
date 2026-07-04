// Toast ownership (endgame architecture batch 5a, "Local state colocation
// list": ToastProvider with a separate stable ToastActions context).
//
// Two contexts by design: the actions context value is created once and
// never changes, so the many pushToast consumers (AppShell, controllers,
// panels) re-render zero times per toast; only the viewport host consumes
// the volatile toast list. The provider wraps AppShell from App.tsx; the
// viewport itself renders wherever the shell places <ToastViewportHost/>
// (inside its I18nProvider — the viewport translates its dismiss label).

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { ToastViewport, type ToastItem, type ToastTone } from "./toast";

const TRANSIENT_STATUS_MS = 3200;
const ERROR_TOAST_MS = 4400;

export type PushToast = (
  message: string,
  tone?: ToastTone,
  durationMs?: number,
) => void;

export interface ToastActions {
  pushToast: PushToast;
  dismissToast: (id: number) => void;
}

const ToastActionsContext = createContext<ToastActions | null>(null);
const ToastStateContext = createContext<ToastItem[]>([]);

export function useToastActions(): ToastActions {
  const actions = useContext(ToastActionsContext);
  if (!actions) {
    throw new Error("ToastActionsContext is not provided");
  }
  return actions;
}

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const toastSequenceRef = useRef(1);
  const toastTimeoutsRef = useRef<Record<number, number>>({});

  const dismissToast = useCallback((id: number) => {
    const timeoutId = toastTimeoutsRef.current[id];
    if (timeoutId) {
      window.clearTimeout(timeoutId);
      delete toastTimeoutsRef.current[id];
    }
    setToasts((current) => current.filter((toast) => toast.id !== id));
  }, []);

  const pushToast = useCallback<PushToast>(
    (
      message,
      tone = "info",
      durationMs = tone === "error" ? ERROR_TOAST_MS : TRANSIENT_STATUS_MS,
    ) => {
      const normalizedMessage = message.trim();
      if (!normalizedMessage) {
        return;
      }

      const id = toastSequenceRef.current;
      toastSequenceRef.current += 1;
      setToasts((current) => [
        ...current.slice(-2),
        { id, message: normalizedMessage, tone },
      ]);
      const timeoutId = window.setTimeout(() => {
        delete toastTimeoutsRef.current[id];
        setToasts((current) => current.filter((toast) => toast.id !== id));
      }, durationMs);
      toastTimeoutsRef.current[id] = timeoutId;
    },
    [],
  );

  useEffect(() => {
    return () => {
      Object.values(toastTimeoutsRef.current).forEach((timeoutId) => {
        window.clearTimeout(timeoutId);
      });
      toastTimeoutsRef.current = {};
    };
  }, []);

  const actions = useMemo<ToastActions>(
    () => ({ pushToast, dismissToast }),
    [dismissToast, pushToast],
  );

  return (
    <ToastActionsContext.Provider value={actions}>
      <ToastStateContext.Provider value={toasts}>
        {children}
      </ToastStateContext.Provider>
    </ToastActionsContext.Provider>
  );
}

/**
 * Renders the toast list. Placed by the shell inside its I18nProvider (the
 * viewport translates its dismiss label); only this host re-renders when
 * toasts change.
 */
export function ToastViewportHost() {
  const toasts = useContext(ToastStateContext);
  const { dismissToast } = useToastActions();
  return <ToastViewport onDismiss={dismissToast} toasts={toasts} />;
}
