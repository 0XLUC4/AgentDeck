import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useEffect, useRef } from "react";
import { forwardsInput } from "../../lib/detach";
import { fileRefs, imageExt } from "../../lib/fileRefs";
import { savePasteImage } from "../../lib/ipc";
import { deck, useDeck } from "../../store/deck";
import { attachOutput, reportSize, sendInput } from "../../store/runtime";
import { DEFAULT_FONT_FAMILY, useSettings } from "../../store/settings";
import { isDetached } from "../Detached/host";

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
      fontFamily: useSettings.getState().fontFamily.trim() || DEFAULT_FONT_FAMILY,
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
      // Ctrl+V / Ctrl+Shift+V: let the webview paste (text, or an image via onPaste) instead of sending ^V.
      if (e.ctrlKey && !e.altKey && e.key.toLowerCase() === "v") return false;
      const zoom = e.ctrlKey && !e.altKey ? ZOOM_KEYS[e.key] : undefined;
      if (zoom === undefined) return true;
      if (e.type === "keydown") deck.zoom(paneId, zoom);
      e.preventDefault(); // also blocks the webview's own page zoom
      return false;
    });

    const detach = attachOutput(paneId, (data) => term.write(data));
    // Only the owner window forwards: xterm auto-replies to terminal queries, so a second forwarder doubles them.
    const owner = () => forwardsInput(false, isDetached(paneId));
    const input = term.onData((data) => owner() && sendInput(paneId, data));
    let resizeTimer: ReturnType<typeof setTimeout> | undefined;
    const refit = () => {
      if (host.clientWidth === 0 || host.clientHeight === 0) return; // hidden
      fit.fit();
      clearTimeout(resizeTimer);
      resizeTimer = setTimeout(() => owner() && reportSize(paneId, term.cols, term.rows), 100);
    };
    const observer = new ResizeObserver(refit);
    observer.observe(host);
    viewRef.current = { term, refit };

    // WebView2 can drop the WebGL canvas while the window is in the background: repaint on return.
    const repaint = () => document.visibilityState === "visible" && term.refresh(0, term.rows - 1);
    window.addEventListener("focus", repaint);
    document.addEventListener("visibilitychange", repaint);

    // Files reach the agent as paths in its prompt; term.paste respects the agent's bracketed paste mode.
    const pastePaths = (paths: string[]) => {
      const agent = useDeck.getState().panes[paneId]?.agent ?? "shell";
      term.paste(fileRefs(agent, paths));
      term.focus();
    };
    // Images have no path on the clipboard: save them to a temp file first.
    const onPaste = (e: ClipboardEvent) => {
      const images = [...(e.clipboardData?.files ?? [])].filter((f) => imageExt(f.type));
      if (images.length === 0) return; // text: xterm's own paste handler
      e.preventDefault();
      e.stopPropagation();
      Promise.all(
        images.map(async (f) => savePasteImage(new Uint8Array(await f.arrayBuffer()), imageExt(f.type) ?? "png")),
      )
        .then(pastePaths)
        .catch((err) => deck.toast(`Could not paste the image: ${String(err)}`));
    };
    host.addEventListener("paste", onPaste, true);

    // Native drag & drop gives real file paths (any file, not only images); highlight the pane under the cursor.
    const setDropTarget = (on: boolean) => host.classList.toggle("drop-target", on);
    const unlistenDrop = getCurrentWebview().onDragDropEvent(({ payload }) => {
      if (payload.type === "leave") return setDropTarget(false);
      const at = payload.position.toLogical(window.devicePixelRatio);
      const inside = host.contains(document.elementFromPoint(at.x, at.y));
      setDropTarget(inside && payload.type !== "drop");
      if (payload.type === "drop" && inside && payload.paths.length > 0) pastePaths(payload.paths);
    });

    return () => {
      viewRef.current = null;
      window.removeEventListener("focus", repaint);
      document.removeEventListener("visibilitychange", repaint);
      host.removeEventListener("paste", onPaste, true);
      void unlistenDrop.then((unlisten) => unlisten());
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
