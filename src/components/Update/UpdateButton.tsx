// Title bar update button: a blue dot when a new release is out, a small panel to install it and restart.
import { getVersion } from "@tauri-apps/api/app";
import { CircleArrowUp, RefreshCw } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { checkForUpdate, installAndRelaunch, useUpdate } from "../../lib/updater";
import { IconButton } from "../IconButton/IconButton";

export function UpdateButton() {
  const { update, phase, progress, error } = useUpdate();
  const [open, setOpen] = useState(false);
  const [version, setVersion] = useState("");
  const rootRef = useRef<HTMLSpanElement>(null);
  const busy = phase === "downloading" || phase === "installing";

  useEffect(() => {
    getVersion().then(setVersion, () => undefined);
  }, []);

  // Click outside or Esc closes the panel (but never while installing).
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!busy && !rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open, busy]);

  const label = update ? `Update available: ${update.version}` : "Updates";
  return (
    <span ref={rootRef} className="relative inline-flex">
      <IconButton
        icon={CircleArrowUp}
        label={label}
        hint={update ? `AgentDeck ${update.version} is available` : "Check for updates"}
        aria-pressed={open}
        aria-expanded={open}
        onClick={() => {
          setOpen(!open);
          if (!open && !update && phase !== "checking") checkForUpdate().catch(() => undefined);
        }}
      />
      {update && (
        <span
          className="pointer-events-none absolute -top-0.5 -right-0.5 size-2.5 rounded-full bg-busy ring-2 ring-panel"
          aria-hidden="true"
        />
      )}
      {open && (
        <div
          role="dialog"
          aria-label="Updates"
          className="absolute top-full right-0 z-50 mt-1.5 w-72 rounded-lg border border-line bg-panel p-3 text-[13px] shadow-lg"
        >
          {update ? (
            <>
              <p className="font-medium text-fg">AgentDeck {update.version} is available</p>
              <p className="mt-0.5 text-xs text-subtle">You have {version || "an older version"}.</p>
              {update.body && (
                <p className="mt-2 max-h-32 overflow-y-auto whitespace-pre-line text-xs text-muted">{update.body}</p>
              )}
              {busy ? (
                <div className="mt-3">
                  <div className="h-1 overflow-hidden rounded-full bg-raised">
                    <div
                      className={`h-full bg-busy transition-[width] duration-150 ${progress === null ? "w-1/3 animate-pulse" : ""}`}
                      style={progress === null ? undefined : { width: `${Math.round(progress * 100)}%` }}
                    />
                  </div>
                  <p className="mt-1.5 text-xs text-subtle">
                    {phase === "installing" ? "Installing, AgentDeck will restart…" : "Downloading…"}
                  </p>
                </div>
              ) : (
                <button
                  type="button"
                  onClick={() => void installAndRelaunch(update)}
                  className="mt-3 inline-flex h-8 w-full items-center justify-center rounded-md bg-busy px-3 font-medium text-on-accent transition-opacity duration-150 hover:opacity-90"
                >
                  Update and restart
                </button>
              )}
              <p className="mt-2 text-xs text-subtle">Running agents are stopped when the app restarts.</p>
            </>
          ) : (
            <div className="flex items-center justify-between gap-2">
              <div>
                <p className="font-medium text-fg">
                  {phase === "checking" ? "Checking for updates…" : "You're up to date"}
                </p>
                {version && <p className="mt-0.5 text-xs text-subtle">AgentDeck {version}</p>}
              </div>
              <IconButton
                icon={RefreshCw}
                label="Check again"
                disabled={phase === "checking"}
                onClick={() => checkForUpdate().catch(() => undefined)}
              />
            </div>
          )}
          {error && <p className="mt-2 text-xs text-bad">{error}</p>}
        </div>
      )}
    </span>
  );
}
