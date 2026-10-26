import { Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { historyClear, historyGetEnabled, historySetEnabled } from "../../lib/ipc";
import { button, Row, Toggle } from "../Settings/controls";
import type { Save } from "../Settings/SettingsDialog";

/** Settings rows: terminal history on/off (stored by the backend) and "Clear history". */
export function HistorySettings({ save }: { save: Save }) {
  const [enabled, setEnabled] = useState<boolean | null>(null);

  useEffect(() => {
    historyGetEnabled().then(setEnabled, (err) => console.warn("history_get_enabled failed", err));
  }, []);

  return (
    <>
      <Row
        label="Save terminal history"
        hint="Plain text on this PC, kept for 30 days (Ctrl+Shift+F to search)"
        htmlFor="set-history"
      >
        <Toggle
          id="set-history"
          checked={enabled ?? true}
          onChange={(v) => {
            setEnabled(v);
            save(historySetEnabled(v));
          }}
        />
      </Row>
      <Row label="Clear history" hint="Deletes all saved terminal output">
        <button
          type="button"
          className={button}
          onClick={() => {
            if (window.confirm("Delete all terminal history?")) save(historyClear());
          }}
        >
          <Trash2 size={14} strokeWidth={1.5} aria-hidden="true" />
          Clear
        </button>
      </Row>
    </>
  );
}
