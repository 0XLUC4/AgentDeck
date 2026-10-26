import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Check, Download, FolderOpen, GitBranch, Play, SquareTerminal, X } from "lucide-react";
import { type FormEvent, useEffect, useRef, useState } from "react";
import { AGENT_KINDS, AGENTS } from "../../lib/agents";
import { detectAgents } from "../../lib/ipc";
import { splitArgs } from "../../lib/splitArgs";
import type { AgentInfo, AgentKind } from "../../lib/types";
import { activeWorkspace, FONT_SIZE, folderName, useDeck } from "../../store/deck";
import { createPane } from "../../store/runtime";
import { AgentLogo } from "../AgentLogo/AgentLogo";
import { IconButton } from "../IconButton/IconButton";

const errorMessage = (err: unknown) => (err instanceof Error ? err.message : String(err));

const field =
  "h-9 min-w-0 rounded-md border border-line bg-base px-2.5 text-fg placeholder:text-subtle outline-none transition-colors duration-150 hover:border-line-strong focus:border-accent";

function InstallState({ known, ok }: { known: boolean; ok: boolean }) {
  if (!known) return <span className="text-xs text-subtle">Détection…</span>;
  return ok ? (
    <span className="inline-flex items-center gap-1 text-xs text-ok">
      <Check size={12} strokeWidth={1.5} aria-hidden="true" />
      Installé
    </span>
  ) : (
    <span className="text-xs text-subtle">Non trouvé</span>
  );
}

/** Modal "Ajouter un agent". Native <dialog>: Esc closes, background is inert. */
export function AgentPicker({ onClose }: { onClose: () => void }) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const workspace = useDeck((s) => activeWorkspace(s));
  const [infos, setInfos] = useState<Partial<Record<AgentKind, AgentInfo>> | null>(null);
  const [agent, setAgent] = useState<AgentKind>("claude");
  const [cwd, setCwd] = useState(() => workspace.path || useDeck.getState().lastCwd);
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
        title: folderName(cwd) || AGENTS[agent].name,
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
      className="m-auto w-[min(36rem,92vw)] rounded-xl border border-line bg-panel p-0 text-fg backdrop:bg-gutter/70"
    >
      <form onSubmit={submit} className="flex flex-col">
        <div className="flex items-center justify-between pt-4 pr-3 pl-5">
          <div>
            <h2 id="agent-picker-title" className="text-[15px] font-medium">
              Ajouter un agent
            </h2>
            <p className="text-xs text-subtle">Dans {workspace.name}</p>
          </div>
          <IconButton icon={X} label="Fermer" onClick={onClose} />
        </div>

        <div className="flex flex-col gap-5 p-5">
          <fieldset className="grid grid-cols-2 gap-2">
            <legend className="mb-2 text-xs font-medium text-muted">Agent</legend>
            {AGENT_KINDS.map((kind) => {
              const meta = AGENTS[kind];
              const ok = installed(kind);
              const selected = agent === kind;
              const installUrl = infos?.[kind]?.installUrl;
              return (
                <label
                  key={kind}
                  className={`relative flex items-center gap-3 rounded-lg border p-3 transition-colors duration-150 has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-accent ${
                    ok ? "cursor-pointer" : "opacity-60"
                  } ${selected ? "border-line-strong bg-raised" : "border-line hover:bg-hover/50"}`}
                >
                  <input
                    type="radio"
                    name="agent"
                    value={kind}
                    checked={selected}
                    disabled={!ok}
                    onChange={() => setAgent(kind)}
                    className="sr-only"
                  />
                  <span className="flex size-8 shrink-0 items-center justify-center">
                    <AgentLogo kind={kind} size={22} />
                  </span>
                  <span className="flex min-w-0 flex-col gap-0.5">
                    <span className="font-medium">{meta.name}</span>
                    <InstallState known={infos !== null} ok={ok} />
                  </span>
                  {!ok && installUrl && (
                    <button
                      type="button"
                      onClick={() => openUrl(installUrl).catch((err) => setError(errorMessage(err)))}
                      className="ml-auto inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs text-fg hover:bg-hover"
                    >
                      <Download size={12} strokeWidth={1.5} aria-hidden="true" />
                      Installer
                    </button>
                  )}
                </label>
              );
            })}
          </fieldset>

          <div className="flex flex-col gap-1.5">
            <label htmlFor="agent-cwd" className="text-xs font-medium text-muted">
              Dossier de travail
            </label>
            <div className="flex gap-2">
              <input
                id="agent-cwd"
                value={cwd}
                onChange={(e) => setCwd(e.target.value)}
                placeholder="C:\chemin\du\projet"
                className={`${field} flex-1 font-mono text-xs`}
              />
              <button
                type="button"
                onClick={pickFolder}
                className="inline-flex h-9 items-center gap-1.5 rounded-md border border-line px-3 text-muted transition-colors duration-150 hover:bg-hover hover:text-fg"
              >
                <FolderOpen size={15} strokeWidth={1.5} aria-hidden="true" />
                Parcourir
              </button>
            </div>
          </div>

          <div className="flex flex-col gap-1.5">
            <label htmlFor="agent-args" className="flex items-center gap-1.5 text-xs font-medium text-muted">
              <SquareTerminal size={13} strokeWidth={1.5} aria-hidden="true" />
              Arguments <span className="font-normal text-subtle">(optionnel)</span>
            </label>
            <input
              id="agent-args"
              value={args}
              onChange={(e) => setArgs(e.target.value)}
              placeholder='--model sonnet --append "texte avec espaces"'
              className={`${field} font-mono text-xs`}
            />
          </div>

          <label className="flex cursor-pointer items-start gap-3 rounded-lg border border-line p-3 transition-colors duration-150 hover:bg-hover/50 has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-accent">
            <input
              type="checkbox"
              checked={worktree}
              onChange={(e) => setWorktree(e.target.checked)}
              className="mt-0.5 size-4 accent-[var(--color-accent)]"
            />
            <span className="flex flex-col gap-0.5">
              <span className="flex items-center gap-1.5 font-medium">
                <GitBranch size={14} strokeWidth={1.5} aria-hidden="true" />
                Worktree Git dédié
              </span>
              <span className="text-xs text-subtle">L'agent travaille sur sa propre copie, sans gêner les autres.</span>
            </span>
          </label>

          {error && (
            <p role="alert" className="rounded-md bg-bad/10 px-3 py-2 text-xs text-bad">
              {error}
            </p>
          )}
        </div>

        <div className="flex justify-end gap-2 px-5 pb-5">
          <button
            type="button"
            onClick={onClose}
            className="h-8 rounded-md px-3 text-muted transition-colors duration-150 hover:bg-hover hover:text-fg"
          >
            Annuler
          </button>
          <button
            type="submit"
            className="inline-flex h-8 items-center gap-1.5 rounded-md bg-accent px-3.5 font-medium text-on-accent transition-colors duration-150 hover:bg-accent-hover"
          >
            <Play size={14} strokeWidth={1.5} aria-hidden="true" />
            Lancer
          </button>
        </div>
      </form>
    </dialog>
  );
}
