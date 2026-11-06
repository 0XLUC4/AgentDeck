import {
  Copy,
  ExternalLink,
  type LucideIcon,
  MoreHorizontal,
  Pencil,
  RotateCcw,
  SquareArrowDownLeft,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AGENTS } from "../../lib/agents";
import type { Pane } from "../../store/deck";
import { deck, FONT_SIZE } from "../../store/deck";
import { isBusy, restartPane } from "../../store/runtime";
import { spawnAgent } from "../AgentPicker/AgentPicker";
import { toggleDetached } from "../Detached/host";
import { IconButton } from "../IconButton/IconButton";

/** Tags to tell same-agent panes apart at a glance ("" = none). */
export const PANE_COLORS = ["", "#ef4444", "#f59e0b", "#22c55e", "#3b82f6", "#a855f7", "#ec4899"];

interface Props {
  pane: Pane;
  detached: boolean;
  onRename: () => void;
}

/** "More" menu of the pane header: the less frequent actions, with text labels instead of a row of icons. */
export function PaneMenu({ pane, detached, onRename }: Props) {
  const [at, setAt] = useState<{ top: number; right: number } | null>(null);
  const button = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const close = () => setAt(null);

  useEffect(() => {
    if (!at) return;
    menu.current?.querySelector<HTMLElement>("[role=menuitem]")?.focus();
    const close = () => setAt(null);
    const onDown = (e: MouseEvent) => {
      if (!menu.current?.contains(e.target as Node) && !button.current?.contains(e.target as Node)) close();
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("mousedown", onDown, true);
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("blur", close);
    return () => {
      window.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("blur", close);
    };
  }, [at]);

  const open = () => {
    const rect = button.current?.getBoundingClientRect();
    if (rect) setAt({ top: rect.bottom + 4, right: window.innerWidth - rect.right });
  };
  const run = (action: () => void) => () => {
    close();
    action();
  };
  const restart = () => {
    if (!isBusy(pane.id) || window.confirm(`${AGENTS[pane.agent].name} is still working. Restart anyway?`)) {
      void restartPane(pane.id);
    }
  };

  const items: [LucideIcon, string, () => void, boolean?][] = [
    [Pencil, "Rename", onRename],
    [Copy, "Duplicate agent", () => spawnAgent(pane.agent, pane.cwd, pane.args, pane.worktree)],
    [RotateCcw, "Restart in a new conversation", restart],
    [ZoomIn, "Zoom in (Ctrl+=)", () => deck.zoom(pane.id, 1), pane.fontSize >= FONT_SIZE.max],
    [ZoomOut, "Zoom out (Ctrl+-)", () => deck.zoom(pane.id, -1), pane.fontSize <= FONT_SIZE.min],
    [
      detached ? SquareArrowDownLeft : ExternalLink,
      detached ? "Reattach" : "Detach to a window",
      () => toggleDetached(pane.id),
    ],
  ];

  return (
    <>
      <IconButton
        ref={button}
        icon={MoreHorizontal}
        size="sm"
        label="More actions"
        aria-haspopup="menu"
        aria-expanded={at !== null}
        onClick={() => (at ? close() : open())}
      />
      {at &&
        createPortal(
          <div
            ref={menu}
            role="menu"
            aria-label={`${pane.title} actions`}
            className="fixed z-50 flex min-w-56 flex-col rounded-lg border border-line-strong bg-raised p-1 text-[13px] shadow-lg"
            style={{ top: at.top, right: at.right }}
          >
            {items.map(([Icon, label, action, disabled]) => (
              <button
                key={label}
                type="button"
                role="menuitem"
                disabled={disabled}
                onClick={run(action)}
                className="flex items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-muted outline-none hover:bg-hover hover:text-fg focus-visible:bg-hover focus-visible:text-fg disabled:opacity-40"
              >
                <Icon size={14} strokeWidth={1.5} aria-hidden="true" />
                {label}
              </button>
            ))}
            <div className="mt-1 flex items-center gap-1.5 border-t border-line px-2.5 pt-2 pb-1.5">
              <span className="mr-auto text-xs text-subtle">Color</span>
              {PANE_COLORS.map((color) => (
                <button
                  key={color || "none"}
                  type="button"
                  aria-label={color ? `Tag with ${color}` : "No color"}
                  aria-pressed={(pane.color ?? "") === color}
                  onClick={run(() => deck.updatePane(pane.id, (p) => ({ ...p, color: color || undefined })))}
                  className="size-4 rounded-full border border-line-strong aria-pressed:ring-2 aria-pressed:ring-fg/70"
                  style={{ background: color || "transparent" }}
                />
              ))}
            </div>
          </div>,
          document.body,
        )}
    </>
  );
}
