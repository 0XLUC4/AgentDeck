// Auto-update: checks GitHub releases at startup and every few hours, shared by the title bar button
// and Settings > About.
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { create } from "zustand";

const RECHECK_MS = 6 * 60 * 60 * 1000;

export type UpdatePhase = "idle" | "checking" | "downloading" | "installing" | "error";

interface UpdateState {
  update: Update | null;
  phase: UpdatePhase;
  /** 0..1 while downloading, null when the size is unknown. */
  progress: number | null;
  error: string | null;
  checkedAt: number | null;
}

export const useUpdate = create<UpdateState>(() => ({
  update: null,
  phase: "idle",
  progress: null,
  error: null,
  checkedAt: null,
}));

const message = (err: unknown) => (err instanceof Error ? err.message : String(err));

/** Returns the available update (also kept in the store), or null when already up to date. */
export async function checkForUpdate(): Promise<Update | null> {
  useUpdate.setState({ phase: "checking", error: null });
  try {
    const update = await check();
    useUpdate.setState({ update, phase: "idle", checkedAt: Date.now() });
    return update;
  } catch (err) {
    useUpdate.setState({ phase: "error", error: message(err) });
    throw err;
  }
}

/** Downloads with progress, installs (passive installer on Windows) and restarts the app. */
export async function installAndRelaunch(update: Update | null = useUpdate.getState().update): Promise<void> {
  if (!update) return;
  let total: number | null = null;
  let received = 0;
  useUpdate.setState({ phase: "downloading", progress: null, error: null });
  try {
    await update.downloadAndInstall((event) => {
      if (event.event === "Started") total = event.data.contentLength ?? null;
      else if (event.event === "Progress") {
        received += event.data.chunkLength;
        useUpdate.setState({ progress: total ? Math.min(received / total, 1) : null });
      } else useUpdate.setState({ phase: "installing", progress: 1 });
    });
    await relaunch();
  } catch (err) {
    useUpdate.setState({ phase: "error", error: message(err) });
  }
}

/** Startup check, then every few hours. Network or endpoint errors stay quiet (the button just shows no dot). */
export function checkForUpdateSilently(): void {
  const run = () => checkForUpdate().catch(() => useUpdate.setState({ phase: "idle" }));
  void run();
  setInterval(run, RECHECK_MS);
}
