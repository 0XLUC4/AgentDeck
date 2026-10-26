// Settings rows backed by Tauri plugins rather than settings.json: autostart and updates.
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import type { Update } from "@tauri-apps/plugin-updater";
import { Download, RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";
import { checkForUpdate, installAndRelaunch } from "../../lib/updater";
import { button, ErrorText, errorMessage, Row, Toggle } from "./controls";
import type { Save } from "./SettingsDialog";

/** "Launch at startup": the OS registration is the source of truth, read through the plugin. */
export function AutostartRow({ save }: { save: Save }) {
  const [enabled, setEnabled] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    isEnabled().then(setEnabled, (err) => setError(errorMessage(err)));
  }, []);

  return (
    <>
      <Row label="Launch AgentDeck when Windows starts" htmlFor="set-autostart">
        <Toggle
          id="set-autostart"
          checked={enabled}
          onChange={(v) => save((v ? enable() : disable()).then(() => setEnabled(v)))}
        />
      </Row>
      {error && <ErrorText>Launch at startup unavailable: {error}</ErrorText>}
    </>
  );
}

/** Check -> show the new version -> "Install and restart". */
export function UpdateRow() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const run = async (task: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await task();
    } catch (err) {
      setStatus(null);
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  const onCheck = () =>
    run(async () => {
      setStatus("Checking…");
      const found = await checkForUpdate();
      setUpdate(found);
      setStatus(found ? `Version ${found.version} available` : "AgentDeck is up to date");
    });

  const onInstall = (found: Update) =>
    run(async () => {
      setStatus("Downloading and installing…");
      await installAndRelaunch(found);
    });

  return (
    <>
      <div className="flex items-center justify-between gap-6 py-3">
        <div className="flex min-w-0 flex-col gap-0.5">
          <span className="text-fg">Updates</span>
          <span role="status" className="text-xs text-subtle">
            {status}
          </span>
        </div>
        {update ? (
          <button type="button" disabled={busy} onClick={() => onInstall(update)} className={button}>
            <Download size={14} strokeWidth={1.5} aria-hidden="true" />
            Install and restart
          </button>
        ) : (
          <button type="button" disabled={busy} onClick={onCheck} className={button}>
            <RefreshCw size={14} strokeWidth={1.5} aria-hidden="true" />
            Check for updates
          </button>
        )}
      </div>
      {error && <ErrorText>Update failed: {error}</ErrorText>}
    </>
  );
}
