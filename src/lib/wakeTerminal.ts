import type { Terminal } from "@xterm/xterm";

type Box = { left: number; top: number; right: number; bottom: number; width: number; height: number };

/** True when the box has a size and overlaps the viewport (hidden dockview tabs sit off-screen). */
export function onViewport(box: Box, viewportWidth: number, viewportHeight: number): boolean {
  return (
    box.width > 0 &&
    box.height > 0 &&
    box.right > 0 &&
    box.bottom > 0 &&
    box.left < viewportWidth &&
    box.top < viewportHeight
  );
}

type PausableCore = { _renderService?: { _handleIntersectionChange?: (entry: { isIntersecting: boolean }) => void } };

/**
 * Redraws a terminal that is really on screen. After a minimize/restore WebView2 can skip the IntersectionObserver
 * update xterm relies on, so xterm stays "paused" and drops every refresh (text only came back by selecting it).
 * Returns false (and does nothing) when the terminal is hidden.
 */
export function wakeTerminal(term: Terminal, host: HTMLElement): boolean {
  if (document.visibilityState !== "visible") return false;
  if (!onViewport(host.getBoundingClientRect(), window.innerWidth, window.innerHeight)) return false;
  // shortcut: private xterm API (RenderService pause flag); drop once xterm re-checks visibility itself.
  (term as unknown as { _core?: PausableCore })._core?._renderService?._handleIntersectionChange?.({
    isIntersecting: true,
  });
  term.refresh(0, term.rows - 1);
  return true;
}
