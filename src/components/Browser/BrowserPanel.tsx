// Browser panel of a Claude pane. The page itself is a native child webview (src-tauri/src/browser.rs)
// laid over this panel's body: we only report where the body is, and hide the webview when it is not shown.
import type { IDockviewPanelProps } from "dockview-react";
import { ArrowLeft, ArrowRight, Globe, RotateCw, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { browserBounds, browserClose, browserGo, browserUrl } from "../../lib/ipc";
import { deck, type Pane, useDeck } from "../../store/deck";
import { restartPane } from "../../store/runtime";
import { IconButton } from "../IconButton/IconButton";

/** Dock panel id of a pane's browser. */
export const browserPanelId = (paneId: string) => `browser:${paneId}`;
export const paneOfBrowserPanel = (panelId: string) =>
  panelId.startsWith("browser:") ? panelId.slice("browser:".length) : null;

/** How often the overlay follows its panel (moves, resizes, tab switches, dialogs). */
const FOLLOW_MS = 150;

export function BrowserPanel({ params }: IDockviewPanelProps<{ paneId: string }>) {
  const body = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const paneId = params.paneId;
    let last = "";
    const follow = () => {
      const el = body.current;
      if (!el) return;
      const r = el.getBoundingClientRect();
      // A native webview sits above the whole page: hide it while one of our dialogs is open.
      const visible = r.width > 0 && r.height > 0 && !document.querySelector("[role=dialog]");
      const key = visible ? `${r.x},${r.y},${r.width},${r.height}` : "hidden";
      if (key === last) return;
      last = key;
      browserBounds(paneId, r.x, r.y, r.width, r.height, visible).catch(() => {
        last = ""; // not created yet (the agent is starting): retry on the next tick
      });
    };
    follow();
    const timer = setInterval(follow, FOLLOW_MS);
    return () => {
      clearInterval(timer);
      browserBounds(paneId, 0, 0, 0, 0, false).catch(() => undefined);
    };
  }, [params.paneId]);

  return (
    <div ref={body} className="flex h-full items-center justify-center bg-base text-xs text-subtle">
      Starting the browser…
    </div>
  );
}

/** Dockview tab of the browser panel: address bar and controls. */
export function BrowserHeader({ paneId }: { paneId: string }) {
  const pane = useDeck((s) => s.panes[paneId]);
  const [url, setUrl] = useState("");
  const [editing, setEditing] = useState(false);

  useEffect(() => {
    if (editing) return;
    const read = () =>
      browserUrl(paneId)
        .then(setUrl)
        .catch(() => undefined);
    read();
    const timer = setInterval(read, 1000);
    return () => clearInterval(timer);
  }, [paneId, editing]);

  const go = (to: string) => browserGo(paneId, to).catch((err) => deck.toast(`Browser: ${err}`));

  return (
    <header className="flex h-full min-w-0 items-center gap-1.5 px-2.5 text-[13px] text-fg">
      <Globe size={14} strokeWidth={1.5} aria-hidden="true" />
      <span className="shrink-0 font-medium">{pane ? `${pane.title} · Browser` : "Browser"}</span>
      <IconButton icon={ArrowLeft} size="sm" label="Back" onClick={() => go("back")} />
      <IconButton icon={ArrowRight} size="sm" label="Forward" onClick={() => go("forward")} />
      <IconButton icon={RotateCw} size="sm" label="Reload" onClick={() => go("reload")} />
      <form
        className="min-w-0 flex-1"
        onSubmit={(e) => {
          e.preventDefault();
          setEditing(false);
          void go(url);
        }}
      >
        <input
          className="w-full min-w-0 rounded border border-line bg-base px-2 py-0.5 text-xs text-fg outline-none focus:border-accent"
          value={url}
          aria-label="Address"
          spellCheck={false}
          onFocus={() => setEditing(true)}
          onBlur={() => setEditing(false)}
          onChange={(e) => setUrl(e.target.value)}
          // The header is also the drag handle of the panel.
          onMouseDown={(e) => e.stopPropagation()}
        />
      </form>
      {pane && <IconButton icon={X} size="sm" label="Close browser" onClick={() => removeBrowser(pane)} />}
    </header>
  );
}

/** Closes the browser for good and restarts the agent without its browser tools. */
export function removeBrowser(pane: Pane): void {
  if (!window.confirm(`Close ${pane.title}'s browser? The agent restarts.`)) return;
  deck.updatePane(pane.id, (p) => ({ ...p, browser: false }));
  browserClose(pane.id).finally(() => void restartPane(pane.id));
}
