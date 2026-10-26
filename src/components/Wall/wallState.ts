// Wall mode state and pure helpers (no Tauri imports, so they stay testable).
import { create } from "zustand";
import { type ActivityItem, activityGroups, type Pane, type Tab, type Workspace } from "../../store/deck";

export const useWall = create<{ open: boolean; allWorkspaces: boolean }>(() => ({
  open: false,
  allWorkspaces: false,
}));

export const toggleWall = () => useWall.setState((s) => ({ open: !s.open }));

/** Panes shown on the wall, most urgent first (waiting, error, done, working, idle, stopped), oldest status first. */
export function wallItems(
  tabs: Tab[],
  workspaces: Workspace[],
  panes: Record<string, Pane>,
  workspaceId: string | null,
): ActivityItem[] {
  return activityGroups(tabs, workspaces, panes)
    .flatMap((group) => group.items)
    .filter((item) => workspaceId === null || item.workspace.id === workspaceId);
}

// CSI, OSC (BEL or ST terminated), charset selection, then any other two-byte ESC sequence (ESC 7, ESC =...).
// biome-ignore lint/suspicious/noControlCharactersInRegex: matching terminal escape codes is the point
const ANSI = /\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[()#][0-9A-Za-z]|\x1b[0-~]/g;
// biome-ignore lint/suspicious/noControlCharactersInRegex: leftover control characters are dropped
const CONTROL = /[\x00-\x08\x0b-\x1f\x7f]/g;

export const stripAnsi = (text: string) => text.replace(ANSI, "");

/**
 * Last non-blank lines of raw terminal output as plain text. A bare \r rewinds the line, so the last segment wins.
 * Note: cursor-addressed TUI redraws are not emulated (may show stale lines); read xterm's buffer if that matters.
 */
export function lastLines(raw: string, count = 6): string[] {
  return stripAnsi(raw)
    .split(/\r?\n/)
    .map((line) =>
      (
        line
          .split("\r")
          .filter((segment) => segment !== "")
          .pop() ?? ""
      )
        .replace(CONTROL, "")
        .trimEnd(),
    )
    .filter((line) => line.trim() !== "")
    .slice(-count);
}
