// Auto-update: manual check from Settings > About, plus one silent check at startup.
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { deck } from "../store/deck";

/** Returns the available update, or null when already up to date. */
export const checkForUpdate = (): Promise<Update | null> => check();

/** Downloads, installs (passive installer on Windows) and restarts the app. */
export async function installAndRelaunch(update: Update): Promise<void> {
  await update.downloadAndInstall();
  await relaunch();
}

/** Startup check: only a toast when an update exists; network/endpoint errors are ignored. */
export async function checkForUpdateSilently(): Promise<void> {
  try {
    const update = await check();
    if (update) {
      deck.toast(`AgentDeck ${update.version} is available: install it from Settings › About.`);
    }
  } catch {
    // Endpoint unreachable or not published yet: stay quiet.
  }
}
