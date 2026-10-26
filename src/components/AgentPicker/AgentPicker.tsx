import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { type FormEvent, useEffect, useRef, useState } from "react";
import { AGENT_KINDS, AGENTS } from "../../lib/agents";
import { detectAgents } from "../../lib/ipc";
import { splitArgs } from "../../lib/splitArgs";
import type { AgentInfo, AgentKind } from "../../lib/types";
import { FONT_SIZE, useDeck } from "../../store/deck";
import { createPane } from "../../store/runtime";

const errorMessage = (err: unknown) => (err instanceof Error ? err.message : String(err));

/** Modal "+ Ajouter un agent". Native <dialog>: Esc closes, background is inert. */
export function AgentPicker({ onClose }: { onClose: () => void }) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [infos, setInfos] = useState<Partial<Record<AgentKind, AgentInfo>> | null>(null);
  const [agent, setAgent] = useState<AgentKind>("claude");
  const [cwd, setCwd] = useState(() => useDeck.getState().lastCwd);
  const [worktree, setWorktree] = useState(false);
  const [args, setArgs] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    dialogRef.current?.showModal();
    detectAgents()
      .then((list) => setInfos(Object.fromEntries(list.map((info) => [info.kind, info]))))
      .catch((err) => {
        setInfos({});
        setError(`Détection des agents impossible : ${errorMessage(err)}`);
      });
  }, []);

  // Unknown install state (detection failed/pending) is treated as installed; spawn errors are toasted.
  const installed = (kind: AgentKind) => infos?.[kind]?.installed ?? true;

  async function pickFolder() {
    try {
      const dir = await openDialog({ directory: true, defaultPath: cwd || undefined });
      if (typeof dir === "string") setCwd(dir);
    } catch (err) {
      setError(`Sélection du dossier impossible : ${errorMessage(err)}`);
    }
  }

  function submit(e: FormEvent) {
    e.preventDefault();
    if (!cwd) return setError("Choisissez un dossier de travail.");
    if (!installed(agent)) return setError(`${AGENTS[agent].name} n'est pas installé.`);
    const { activeTabId } = useDeck.getState();
    useDeck.setState({ lastCwd: cwd });
    void createPane(
      {
        id: crypto.randomUUID(),
        agent,
        title: cwd.split(/[\\/]/).filter(Boolean).pop() ?? AGENTS[agent].name,
        cwd,
        args: splitArgs(args),
        worktree,
        completedCount: 0,
        fontSize: FONT_SIZE.default,
      },
      activeTabId,
    );
    onClose();
  }

  return (
    <dialog
      ref={dialogRef}
      onClose={onClose}
      aria-labelledby="agent-picker-title"
      className="m-auto w-[min(40rem,92vw)] rounded-xl border border-zinc-700 bg-zinc-900 p-0 text-zinc-100 backdrop:bg-black/60"
    >
      <form onSubmit={submit} className="flex flex-col gap-4 p-5">
        <h2 id="agent-picker-title" className="text-lg font-semibold">
          Ajouter un agent
        </h2>

        <fieldset className="grid grid-cols-2 gap-3">
          <legend className="sr-only">Agent</legend>
          {AGENT_KINDS.map((kind) => {
            const meta = AGENTS[kind];
            const ok = installed(kind);
            const installUrl = infos?.[kind]?.installUrl;
            return (
              <label
                key={kind}
                className={`flex items-center gap-3 rounded-lg border-2 p-3 ${ok ? "cursor-pointer" : "opacity-50"} ${
                  agent === kind ? "bg-zinc-800" : "border-zinc-700"
                } has-[:focus-visible]:outline has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-blue-500`}
                style={agent === kind ? { borderColor: meta.color } : undefined}
              >
                <input
                  type="radio"
                  name="agent"
                  value={kind}
                  checked={agent === kind}
                  disabled={!ok}
                  onChange={() => setAgent(kind)}
                  className="sr-only"
                />
                <img src={meta.logo} alt="" className="size-10" />
                <span className="flex flex-col">
                  <span className="font-medium">{meta.name}</span>
                  <span className={`text-xs ${ok ? "text-green-400" : "text-zinc-400"}`}>
                    {infos === null ? "Détection…" : ok ? "Installé" : "Non trouvé"}
                  </span>
                  {!ok && installUrl && (
                    <button
                      type="button"
                      onClick={() => openUrl(installUrl).catch((err) => setError(errorMessage(err)))}
                      className="text-left text-xs text-blue-400 underline"
                    >
                      Installer
                    </button>
                  )}
                </span>
              </label>
            );
          })}
        </fieldset>

        <div className="flex flex-col gap-1">
          <span className="text-sm text-zinc-400">Dossier de travail</span>
          <div className="flex gap-2">
            <input
              value={cwd}
              onChange={(e) => setCwd(e.target.value)}
              aria-label="Dossier de travail"
              placeholder="C:\chemin\du\projet"
              className="min-w-0 flex-1 rounded border border-zinc-700 bg-zinc-950 px-2 py-1"
            />
            <button type="button" onClick={pickFolder} className="rounded bg-zinc-700 px-3 py-1 hover:bg-zinc-600">
              Parcourir…
            </button>
          </div>
        </div>

        <label className="flex items-center gap-2 text-sm">
          <input type="checkbox" checked={worktree} onChange={(e) => setWorktree(e.target.checked)} />
          Worktree Git dédié
        </label>

        <label className="flex flex-col gap-1 text-sm">
          <span className="text-zinc-400">Arguments (optionnel)</span>
          <input
            value={args}
            onChange={(e) => setArgs(e.target.value)}
            placeholder='--model sonnet --append "texte avec espaces"'
            className="rounded border border-zinc-700 bg-zinc-950 px-2 py-1 font-mono"
          />
        </label>

        {error && (
          <p role="alert" className="text-sm text-red-400">
            {error}
          </p>
        )}

        <div className="flex justify-end gap-2">
          <button type="button" onClick={onClose} className="rounded px-3 py-1.5 hover:bg-zinc-800">
            Annuler
          </button>
          <button type="submit" className="rounded bg-blue-600 px-3 py-1.5 font-medium hover:bg-blue-500">
            Lancer
          </button>
        </div>
      </form>
    </dialog>
  );
}
