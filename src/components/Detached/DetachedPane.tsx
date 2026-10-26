// Content of a pane-* window: a frameless title bar and the pane's terminal, owner of input and resizes.
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { SquareArrowDownLeft, X, ZoomIn, ZoomOut } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { AGENTS } from "../../lib/agents";
import {
  geometryKey,
  PANE_META,
  PANE_REMOTE,
  REPLAY,
  REPLAY_REQUEST,
  type RemoteAction,
  type RemoteBody,
  type Replay,
  type ReplayRequest,
  replayGate,
} from "../../lib/detach";
import { onPaneOutput } from "../../lib/ipc";
import { FONT_SIZE, type Pane } from "../../store/deck";
import { AgentLogo } from "../AgentLogo/AgentLogo";
import { IconButton } from "../IconButton/IconButton";
import { StatusBadge } from "../StatusBadge/StatusBadge";

// Mirrors of TerminalPane.tsx (importing it would pull the main window's runtime into this window).
const THEME = { background: "#100e0c", foreground: "#ebe7e2", cursor: "#f1e7d2", selectionBackground: "#3b3732" };
const ZOOM_KEYS: Record<string, number | "reset"> = { "=": 1, "+": 1, "-": -1, "0": "reset" };

const appWindow = getCurrentWindow();
const warn = (what: string) => (err: unknown) => console.warn(`${what} failed`, err);

const sendRemote = (paneId: string, body: RemoteBody) =>
  emit(PANE_REMOTE, { paneId, ...body } satisfies RemoteAction).catch(warn("pane-remote"));

/** Closing the window reattaches the pane (the main window listens for its destruction). Saves size/position first. */
async function saveGeometry(paneId: string): Promise<void> {
  const scale = await appWindow.scaleFactor();
  const [pos, size] = await Promise.all([appWindow.outerPosition(), appWindow.innerSize()]);
  const { x, y } = pos.toLogical(scale);
  const { width, height } = size.toLogical(scale);
  try {
    localStorage.setItem(geometryKey(paneId), JSON.stringify({ x, y, width, height }));
  } catch (err) {
    warn("save window geometry")(err);
  }
}

function TitleBar({ pane, send }: { pane: Pane | null; send: (body: RemoteBody) => void }) {
  const close = () => appWindow.close().catch(warn("close"));
  return (
    <header
      data-tauri-drag-region
      className="flex h-9 shrink-0 select-none items-center gap-2 border-b border-line bg-panel pl-2.5 text-[13px]"
    >
      {pane && <AgentLogo kind={pane.agent} size={14} />}
      <span data-tauri-drag-region className="truncate font-medium text-fg">
        {pane ? pane.title || AGENTS[pane.agent].name : "Chargement…"}
      </span>
      {pane && <StatusBadge status={pane.status} />}
      {pane?.exited && (
        <span data-tauri-drag-region className="truncate text-xs text-subtle">
          Processus terminé : R pour relancer
        </span>
      )}
      <div data-tauri-drag-region className="h-full min-w-6 flex-1" />
      <div className="flex items-center gap-0.5 pr-1" role="toolbar" aria-label="Actions du pane">
        <IconButton
          icon={ZoomOut}
          size="sm"
          label="Dézoomer"
          hint="Dézoomer (Ctrl+-)"
          onClick={() => send({ kind: "zoom", delta: -1 })}
          disabled={!pane || pane.fontSize <= FONT_SIZE.min}
        />
        <IconButton
          icon={ZoomIn}
          size="sm"
          label="Zoomer"
          hint="Zoomer (Ctrl+=, Ctrl+0 pour réinitialiser)"
          onClick={() => send({ kind: "zoom", delta: 1 })}
          disabled={!pane || pane.fontSize >= FONT_SIZE.max}
        />
        <IconButton
          icon={SquareArrowDownLeft}
          size="sm"
          label="Rattacher"
          hint="Rattacher à la fenêtre principale"
          onClick={close}
        />
      </div>
      <button
        type="button"
        aria-label="Fermer la fenêtre (rattache le pane)"
        title="Fermer (rattache le pane)"
        className="inline-flex h-full w-[46px] items-center justify-center text-muted transition-colors duration-150 hover:bg-close hover:text-fg focus-visible:-outline-offset-2"
        onClick={close}
      >
        <X size={16} strokeWidth={1.5} aria-hidden="true" />
      </button>
    </header>
  );
}

/** The detached window owns the pane's input: it forwards keystrokes, terminal replies and its size to the main window. */
export function DetachedPane({ paneId }: { paneId: string }) {
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<{ term: Terminal; refit: () => void } | null>(null);
  const [pane, setPane] = useState<Pane | null>(null);
  const generation = useRef<number | null>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const remote = (body: RemoteBody) => sendRemote(paneId, body);
    const term = new Terminal({ cursorBlink: true, scrollback: 5000, theme: THEME, fontSize: FONT_SIZE.default });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => webgl.dispose());
      term.loadAddon(webgl);
    } catch (err) {
      console.warn("WebGL unavailable, using DOM renderer", err);
    }
    term.attachCustomKeyEventHandler((e) => {
      const zoom = e.ctrlKey && !e.altKey ? ZOOM_KEYS[e.key] : undefined;
      if (zoom === undefined) return true;
      if (e.type === "keydown") remote({ kind: "zoom", delta: zoom });
      e.preventDefault();
      return false;
    });
    const input = term.onData((data) => remote({ kind: "input", data }));

    let resizeTimer: ReturnType<typeof setTimeout> | undefined;
    const refit = () => {
      if (host.clientWidth === 0 || host.clientHeight === 0) return;
      fit.fit();
      clearTimeout(resizeTimer);
      resizeTimer = setTimeout(() => remote({ kind: "resize", cols: term.cols, rows: term.rows }), 100);
    };
    const observer = new ResizeObserver(refit);
    observer.observe(host);
    viewRef.current = { term, refit };

    // Subscribe to live output first (held by the gate), then ask the main window for the scrollback.
    const gate = replayGate((data) => term.write(data));
    let disposed = false;
    let unlisten: UnlistenFn[] = [];
    Promise.all([
      onPaneOutput(paneId, gate.output),
      listen<Replay>(REPLAY, ({ payload }) => {
        if (payload.paneId !== paneId) return;
        term.options.fontFamily = payload.fontFamily;
        refit();
        generation.current = payload.pane.generation;
        setPane(payload.pane);
        gate.replay(payload.text);
        term.focus();
      }),
      listen<Pane>(PANE_META, ({ payload }) => {
        if (payload.id === paneId) setPane(payload);
      }),
      appWindow.onFocusChanged(({ payload: focused }) => {
        if (focused) remote({ kind: "focus" });
      }),
      appWindow.onCloseRequested(() => saveGeometry(paneId).catch(warn("save window geometry"))),
    ])
      .then((fns) => {
        if (disposed) {
          for (const fn of fns) fn();
          return;
        }
        unlisten = fns;
        const request: ReplayRequest = { paneId, windowLabel: appWindow.label };
        return emit(REPLAY_REQUEST, request);
      })
      .catch(warn("detached pane setup"));

    return () => {
      disposed = true;
      viewRef.current = null;
      for (const fn of unlisten) fn();
      observer.disconnect();
      clearTimeout(resizeTimer);
      input.dispose();
      term.dispose();
    };
  }, [paneId]);

  // Restart: the main window cleared its buffer; start from a clean screen too.
  useEffect(() => {
    if (!pane || generation.current === pane.generation) return;
    generation.current = pane.generation;
    viewRef.current?.term.reset();
  }, [pane]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view || !pane || view.term.options.fontSize === pane.fontSize) return;
    view.term.options.fontSize = pane.fontSize;
    view.refit();
  }, [pane]);

  const title = pane ? pane.title || AGENTS[pane.agent].name : "";
  useEffect(() => {
    if (title) appWindow.setTitle(title).catch(warn("setTitle"));
  }, [title]);

  return (
    <div className="flex h-full flex-col bg-base text-fg">
      <TitleBar pane={pane} send={(body) => sendRemote(paneId, body)} />
      <div ref={hostRef} className="min-h-0 flex-1 overflow-hidden bg-base py-1 pl-2" />
    </div>
  );
}
