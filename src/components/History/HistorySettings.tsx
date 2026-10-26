import { Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { historyClear, historyGetEnabled, historySetEnabled } from "../../lib/ipc";
import { button, Row, Toggle } from "../Settings/controls";
import type { Save } from "../Settings/SettingsDialog";

/** Réglages rows: terminal history on/off (stored by the backend) and "Vider l'historique". */
export function HistorySettings({ save }: { save: Save }) {
  const [enabled, setEnabled] = useState<boolean | null>(null);

  useEffect(() => {
    historyGetEnabled().then(setEnabled, (err) => console.warn("history_get_enabled failed", err));
  }, []);

  return (
    <>
      <Row
        label="Enregistrer l'historique des terminaux"
        hint="Texte brut sur ce PC, conservé 30 jours (Ctrl+Maj+F pour chercher)"
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
      <Row label="Vider l'historique" hint="Supprime toute la sortie enregistrée des terminaux">
        <button
          type="button"
          className={button}
          onClick={() => {
            if (window.confirm("Supprimer tout l'historique des terminaux ?")) save(historyClear());
          }}
        >
          <Trash2 size={14} strokeWidth={1.5} aria-hidden="true" />
          Vider
        </button>
      </Row>
    </>
  );
}
