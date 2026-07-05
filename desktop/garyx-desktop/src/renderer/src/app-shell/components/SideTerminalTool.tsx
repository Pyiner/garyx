// SideTerminalTool: the dock's Terminal tool tab (extracted from
// SideToolsPanel in the 2026-07 perf round). xterm and its CSS are heavy
// and strictly tab-scoped, so this module is React.lazy-loaded — it keeps
// ~90 xterm modules out of the boot bundle.

import { useEffect, useRef, useState } from "react";

import { FitAddon } from "@xterm/addon-fit";
import { Terminal as XTermTerminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";

import type {
  DesktopTerminalEvent,
  DesktopTerminalState,
} from "@shared/contracts";
import { useI18n } from "../../i18n";

const MAX_RENDERER_TERMINAL_OUTPUT_LENGTH = 160_000;

function appendTerminalOutput(output: string, data: string): string {
  const nextOutput = `${output}${data}`;
  return nextOutput.length > MAX_RENDERER_TERMINAL_OUTPUT_LENGTH
    ? nextOutput.slice(nextOutput.length - MAX_RENDERER_TERMINAL_OUTPUT_LENGTH)
    : nextOutput;
}

function activeTerminalSession(state: DesktopTerminalState | null) {
  if (!state?.activeSessionId) {
    return null;
  }
  return state.sessions.find((session) => session.id === state.activeSessionId) || null;
}

export function SideTerminalTool({ cwd }: { cwd?: string | null }) {
  const { t } = useI18n();
  const [state, setState] = useState<DesktopTerminalState | null>(null);
  const [creating, setCreating] = useState(false);
  const [terminalReady, setTerminalReady] = useState(false);
  const terminalHostRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<XTermTerminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const renderedSessionIdRef = useRef<string | null>(null);
  const renderedOutputRef = useRef("");
  const activeSessionRef = useRef<ReturnType<typeof activeTerminalSession>>(null);
  const session = activeTerminalSession(state);

  useEffect(() => {
    activeSessionRef.current = session;
  }, [session]);

  useEffect(() => {
    const node = terminalHostRef.current;
    if (!node) {
      return;
    }
    const terminal = new XTermTerminal({
      allowProposedApi: false,
      convertEol: true,
      cursorBlink: true,
      fontFamily: '"SFMono-Regular", "Cascadia Code", Menlo, Monaco, Consolas, monospace',
      fontSize: 12,
      lineHeight: 1.24,
      scrollback: 3_000,
      theme: {
        background: "#ffffff",
        foreground: "#1a1c1f",
        cursor: "#1a1c1f",
        selectionBackground: "#cfe8ff",
        black: "#1a1c1f",
        blue: "#2563eb",
        brightBlack: "#7c7f85",
        brightBlue: "#1d4ed8",
        brightCyan: "#0e7490",
        brightGreen: "#15803d",
        brightMagenta: "#7c3aed",
        brightRed: "#b91c1c",
        brightWhite: "#334155",
        brightYellow: "#b45309",
        cyan: "#0891b2",
        green: "#16a34a",
        magenta: "#9333ea",
        red: "#dc2626",
        white: "#5f6368",
        yellow: "#ca8a04",
      },
    });
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(node);
    terminalRef.current = terminal;
    fitAddonRef.current = fitAddon;

    const resizeToHost = () => {
      try {
        fitAddon.fit();
        const active = activeSessionRef.current;
        if (active?.running) {
          void window.garyxDesktop.resizeTerminalSession({
            sessionId: active.id,
            cols: terminal.cols,
            rows: terminal.rows,
          });
        }
      } catch {
        // The terminal can briefly report a zero-sized host while tabs switch.
      }
    };

    const dataDisposable = terminal.onData((data) => {
      const active = activeSessionRef.current;
      if (!active?.running) {
        return;
      }
      void window.garyxDesktop.writeTerminalInput({
        sessionId: active.id,
        data,
      });
    });
    const resizeDisposable = terminal.onResize(({ cols, rows }) => {
      const active = activeSessionRef.current;
      if (active?.running) {
        void window.garyxDesktop.resizeTerminalSession({
          sessionId: active.id,
          cols,
          rows,
        });
      }
    });
    const observer = new ResizeObserver(resizeToHost);
    observer.observe(node);
    requestAnimationFrame(resizeToHost);
    setTerminalReady(true);

    return () => {
      setTerminalReady(false);
      observer.disconnect();
      dataDisposable.dispose();
      resizeDisposable.dispose();
      terminal.dispose();
      terminalRef.current = null;
      fitAddonRef.current = null;
      renderedSessionIdRef.current = null;
      renderedOutputRef.current = "";
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    const handleEvent = (event: DesktopTerminalEvent) => {
      if (disposed) {
        return;
      }
      if (event.type === "state") {
        setState(event.state);
        return;
      }
      if (activeSessionRef.current?.id === event.sessionId) {
        terminalRef.current?.write(event.data);
        renderedSessionIdRef.current = event.sessionId;
        renderedOutputRef.current = appendTerminalOutput(renderedOutputRef.current, event.data);
      }
      setState((current) => {
        if (!current) {
          return current;
        }
        return {
          ...current,
          sessions: current.sessions.map((entry) =>
            entry.id === event.sessionId
              ? {
                  ...entry,
                  output: appendTerminalOutput(entry.output, event.data),
                  updatedAt: new Date().toISOString(),
                }
              : entry,
          ),
        };
      });
    };
    void window.garyxDesktop.listTerminalState().then((nextState) => {
      if (!disposed) {
        setState(nextState);
      }
    });
    window.garyxDesktop.subscribeTerminalEvents(handleEvent);
    return () => {
      disposed = true;
      window.garyxDesktop.unsubscribeTerminalEvents(handleEvent);
    };
  }, []);

  async function createSession() {
    if (creating) {
      return;
    }
    setCreating(true);
    const terminal = terminalRef.current;
    await window.garyxDesktop
      .createTerminalSession({
        cwd,
        cols: terminal?.cols,
        rows: terminal?.rows,
      })
      .then(setState)
      .finally(() => setCreating(false));
  }

  useEffect(() => {
    if (state === null || state.sessions.length > 0 || creating || !terminalReady) {
      return;
    }
    void createSession();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [creating, state, terminalReady]);

  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal || !terminalReady) {
      return;
    }
    if (!session) {
      terminal.clear();
      renderedSessionIdRef.current = null;
      renderedOutputRef.current = "";
      return;
    }
    if (renderedSessionIdRef.current !== session.id) {
      terminal.reset();
      terminal.write(session.output);
      renderedSessionIdRef.current = session.id;
      renderedOutputRef.current = session.output;
      return;
    }
    const rendered = renderedOutputRef.current;
    if (session.output.startsWith(rendered)) {
      const delta = session.output.slice(rendered.length);
      if (delta) {
        terminal.write(delta);
      }
    } else {
      terminal.reset();
      terminal.write(session.output);
    }
    renderedOutputRef.current = session.output;
  }, [session?.id, session?.output, terminalReady]);

  // The side tools panel already manages tabs; the terminal body stays a
  // single session with no inner session chrome. Closing the exited session
  // lets the auto-create effect start a fresh one.
  function restartExitedSession() {
    const exited = activeSessionRef.current;
    if (!exited || exited.running) {
      return;
    }
    void window.garyxDesktop
      .closeTerminalSession({ sessionId: exited.id })
      .then(setState);
  }

  return (
    <div className="side-tool-terminal">
      <div
        aria-label={t("Terminal input")}
        className="side-tool-terminal-output"
        ref={terminalHostRef}
      />
      {creating ? <div className="side-tool-terminal-status">{t("Starting…")}</div> : null}
      {session && !session.running && !creating ? (
        <button
          className="side-tool-terminal-status side-tool-terminal-restart"
          onClick={restartExitedSession}
          type="button"
        >
          {t("Terminal exited · Restart")}
        </button>
      ) : null}
    </div>
  );
}

