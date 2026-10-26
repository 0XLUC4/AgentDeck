import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef } from "react";
import { deck } from "../../store/deck";
import { attachOutput, reportSize, sendInput } from "../../store/runtime";

// Hex mirrors of --color-base / --color-fg in index.css (xterm needs plain colors).
const THEME = { background: "#100e0c", foreground: "#ebe7e2", cursor: "#f1e7d2", selectionBackground: "#3b3732" };

/** Ctrl+<key> zoom shortcuts, handled before xterm sends them to the process. */
const ZOOM_KEYS: Record<string, number | "reset"> = { "=": 1, "+": 1, "-": -1, "0": "reset" };

/** Attaches an xterm view to an existing pane's output. Never spawns: safe to mount twice. */
export function TerminalPane({ paneId, fontSize }: { paneId: string; fontSize: number }) {
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<{ term: Terminal; refit: () => void } | null>(null);
  const initialFontSize = useRef(fontSize);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const term = new Terminal({
      fontFamily: '"Cascadia Code", Consolas, monospace',
      fontSize: initialFontSize.current,
      cursorBlink: true,
      scrollback: 5000,
      theme: THEME,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => webgl.dispose()); // xterm falls back to the DOM renderer
      term.loadAddon(webgl);
    } catch (err) {
      console.warn("WebGL unavailable, using DOM renderer", err);
    }

    term.attachCustomKeyEventHandler((e) => {
      const zoom = e.ctrlKey && !e.altKey ? ZOOM_KEYS[e.key] : undefined;
      if (zoom === undefined) return true;
      if (e.type === "keydown") deck.zoom(paneId, zoom);
      e.preventDefault(); // also blocks the webview's own page zoom
      return false;
    });

    const detach = attachOutput(paneId, (data) => term.write(data));
    const input = term.onData((data) => sendInput(paneId, data));
    let resizeTimer: ReturnType<typeof setTimeout> | undefined;
    const refit = () => {
      if (host.clientWidth === 0 || host.clientHeight === 0) return; // hidden
      fit.fit();
      clearTimeout(resizeTimer);
      resizeTimer = setTimeout(() => reportSize(paneId, term.cols, term.rows), 100);
    };
    const observer = new ResizeObserver(refit);
    observer.observe(host);
    viewRef.current = { term, refit };

    return () => {
      viewRef.current = null;
      observer.disconnect();
      clearTimeout(resizeTimer);
      detach();
      input.dispose();
      term.dispose();
    };
  }, [paneId]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view || view.term.options.fontSize === fontSize) return;
    view.term.options.fontSize = fontSize;
    view.refit();
  }, [fontSize]);

  return <div ref={hostRef} className="min-h-0 flex-1 overflow-hidden bg-base py-1 pl-2" />;
}
