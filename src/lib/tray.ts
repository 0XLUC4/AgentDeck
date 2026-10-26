// Keeps the Rust side in sync with the UI: tray tooltip (waiting count) and the "close to tray" flag.
import { useEffect } from "react";
import { activityKey, useDeck } from "../store/deck";
import { useSettings } from "../store/settings";
import { setCloseToTray, setTrayStatus } from "./ipc";

export function useTraySync() {
  const waiting = useDeck((s) => Object.values(s.panes).filter((p) => activityKey(p) === "waiting").length);
  const closeToTray = useSettings((s) => s.closeToTray);

  useEffect(() => {
    setTrayStatus(waiting).catch((err) => console.warn("tray status failed", err));
  }, [waiting]);

  useEffect(() => {
    setCloseToTray(closeToTray).catch((err) => console.warn("close-to-tray failed", err));
  }, [closeToTray]);
}
