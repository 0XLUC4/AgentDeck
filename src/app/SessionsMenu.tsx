import { useRef } from "react";
import { deck, useDeck } from "../store/deck";
import { reopenSession } from "../store/runtime";

const dateFormat = new Intl.DateTimeFormat("fr-FR", { dateStyle: "short", timeStyle: "short" });

/** Dropdown of archived sessions: reopen (respawns its agents) or delete. */
export function SessionsMenu() {
  const archived = useDeck((s) => s.archived);
  const detailsRef = useRef<HTMLDetailsElement>(null);
  const close = () => detailsRef.current?.removeAttribute("open");

  return (
    <details ref={detailsRef} className="relative shrink-0">
      <summary className="cursor-pointer list-none rounded-md px-2 py-1 text-sm text-zinc-300 hover:bg-zinc-800">
        Sessions ▾
      </summary>
      <div className="absolute top-full left-0 z-50 mt-1 w-80 rounded-lg border border-zinc-700 bg-zinc-900 p-2 shadow-xl">
        {archived.length === 0 ? (
          <p className="p-2 text-sm text-zinc-400">Aucune session archivée.</p>
        ) : (
          <ul className="flex flex-col gap-1">
            {archived.map((s) => (
              <li key={s.id} className="flex items-center gap-2 rounded p-2 hover:bg-zinc-800">
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm">{s.name}</span>
                  <span className="text-xs text-zinc-400">
                    {dateFormat.format(s.savedAt)} · {s.panes.length} agent(s)
                  </span>
                </span>
                <button
                  type="button"
                  className="rounded bg-blue-600 px-2 py-0.5 text-xs hover:bg-blue-500"
                  onClick={() => {
                    reopenSession(s.id);
                    close();
                  }}
                >
                  Rouvrir
                </button>
                <button
                  type="button"
                  className="rounded px-2 py-0.5 text-xs text-red-400 hover:bg-zinc-700"
                  aria-label={`Supprimer la session ${s.name}`}
                  onClick={() => window.confirm(`Supprimer « ${s.name} » ?`) && deck.deleteArchived(s.id)}
                >
                  Supprimer
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </details>
  );
}
