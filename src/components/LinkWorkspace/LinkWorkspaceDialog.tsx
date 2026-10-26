import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  AppWindow,
  Code2,
  Folder,
  FolderOpen,
  GitBranch,
  type LucideIcon,
  RefreshCw,
  TriangleAlert,
  X,
} from "lucide-react";
import { type FormEvent, type KeyboardEvent, useCallback, useEffect, useRef, useState } from "react";
import { inspectFolder, listOpenProjects } from "../../lib/ipc";
import type { FolderInfo, OpenProject } from "../../lib/types";
import { deck, folderName, useDeck, workspaceForPath } from "../../store/deck";
import { IconButton } from "../IconButton/IconButton";

type Source = "apps" | "git" | "folder";

const SOURCES: { id: Source; label: string; icon: LucideIcon }[] = [
  { id: "apps", label: "Application ouverte", icon: AppWindow },
  { id: "git", label: "Projet Git", icon: GitBranch },
  { id: "folder", label: "Dossier existant", icon: Folder },
];

/** What will be linked. `git` is undefined when unknown or not a repository. */
interface Choice {
  path: string;
  name: string;
  git?: { branch: string | null; remote: string | null };
}

const errorMessage = (err: unknown) => (err instanceof Error ? err.message : String(err));
const appIcon = (app: string): LucideIcon => (/explor/i.test(app) ? FolderOpen : Code2);

function groupByApp(projects: OpenProject[]): [string, OpenProject[]][] {
  const groups = new Map<string, OpenProject[]>();
  for (const p of projects) groups.set(p.app, [...(groups.get(p.app) ?? []), p]);
  return [...groups];
}

function GitLine({ branch, remote }: { branch: string | null; remote: string | null }) {
  return (
    <span className="flex min-w-0 items-center gap-1.5 text-xs text-muted">
      <GitBranch size={13} strokeWidth={1.5} className="shrink-0" aria-hidden="true" />
      <span className="shrink-0 text-fg">{branch ?? "HEAD détachée"}</span>
      {remote && <span className="truncate text-subtle">{remote}</span>}
    </span>
  );
}

function OpenApps({ selected, onPick }: { selected: string | null; onPick: (c: Choice) => void }) {
  const [projects, setProjects] = useState<OpenProject[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    setProjects(null);
    setError(null);
    listOpenProjects()
      .then(setProjects)
      .catch((err) => {
        setProjects([]);
        setError(errorMessage(err));
      });
  }, []);

  useEffect(refresh, [refresh]);

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <p className="text-xs text-subtle">Projets ouverts dans vos éditeurs et l'Explorateur.</p>
        <IconButton icon={RefreshCw} label="Actualiser" onClick={refresh} disabled={projects === null} />
      </div>
      <div className="scroll-thin max-h-64 overflow-y-auto" aria-busy={projects === null}>
        {projects === null ? (
          <ul className="flex flex-col gap-1" aria-label="Chargement">
            {[0, 1, 2].map((i) => (
              <li key={i} className="flex h-11 flex-col justify-center gap-1.5 px-2">
                <span className="h-2.5 w-1/3 rounded bg-raised" />
                <span className="h-2 w-2/3 rounded bg-raised/60" />
              </li>
            ))}
          </ul>
        ) : error ? (
          <p role="alert" className="flex items-start gap-2 py-3 text-xs text-muted">
            <TriangleAlert size={14} strokeWidth={1.5} className="mt-px shrink-0 text-wait" aria-hidden="true" />
            <span>Liste des applications indisponible ({error}). Utilisez « Projet Git » ou « Dossier existant ».</span>
          </p>
        ) : projects.length === 0 ? (
          <p className="py-6 text-center text-muted">Aucune application avec un projet ouvert</p>
        ) : (
          groupByApp(projects).map(([app, list]) => {
            const Icon = appIcon(app);
            return (
              <section key={app} className="mb-2" aria-label={app}>
                <h3 className="flex items-center gap-1.5 px-2 py-1 text-xs font-medium text-subtle">
                  <Icon size={13} strokeWidth={1.5} aria-hidden="true" />
                  {app}
                </h3>
                <ul className="flex flex-col">
                  {list.map((p) => (
                    <li key={`${app}:${p.path}`}>
                      <button
                        type="button"
                        aria-pressed={selected === p.path}
                        onClick={() =>
                          onPick({
                            path: p.path,
                            name: p.name,
                            git: p.isGit ? { branch: null, remote: null } : undefined,
                          })
                        }
                        className="flex w-full items-center gap-3 rounded-md px-2 py-1.5 text-left transition-colors duration-150 hover:bg-hover aria-pressed:bg-raised"
                      >
                        <span className="flex min-w-0 flex-1 flex-col">
                          <span className="truncate font-medium text-fg">{p.name}</span>
                          <span className="truncate font-mono text-xs text-subtle" title={p.path}>
                            {p.path}
                          </span>
                        </span>
                        {p.isGit && (
                          <span
                            className="inline-flex shrink-0 items-center gap-1 text-xs text-muted"
                            title="Dépôt Git"
                          >
                            <GitBranch size={13} strokeWidth={1.5} aria-hidden="true" />
                            Git
                          </span>
                        )}
                      </button>
                    </li>
                  ))}
                </ul>
              </section>
            );
          })
        )}
      </div>
    </div>
  );
}

function FolderSource({
  requireGit,
  choice,
  onPick,
}: {
  requireGit: boolean;
  choice: Choice | null;
  onPick: (c: Choice | null) => void;
}) {
  const [info, setInfo] = useState<FolderInfo | null>(null);
  const [acceptPlain, setAcceptPlain] = useState(false);
  const [inspectError, setInspectError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function browse() {
    let dir: string | string[] | null;
    try {
      dir = await openDialog({ directory: true, title: requireGit ? "Choisir un dépôt Git" : "Choisir un dossier" });
    } catch (err) {
      setInspectError(`Sélection du dossier impossible : ${errorMessage(err)}`);
      return;
    }
    if (typeof dir !== "string") return;
    setBusy(true);
    setInfo(null);
    setAcceptPlain(false);
    setInspectError(null);
    try {
      const inspected = await inspectFolder(dir);
      setInfo(inspected);
      const git = inspected.isGit ? { branch: inspected.branch, remote: inspected.remote } : undefined;
      onPick(requireGit && !inspected.isGit ? null : { path: inspected.path, name: inspected.name, git });
    } catch (err) {
      // Backend without inspect_folder: still link the plain path.
      setInspectError(errorMessage(err));
      onPick({ path: dir, name: folderName(dir) });
    } finally {
      setBusy(false);
    }
  }

  const notGit = requireGit && !acceptPlain && info !== null && !info.isGit;
  return (
    <div className="flex flex-col gap-3">
      <button
        type="button"
        onClick={browse}
        disabled={busy}
        className="inline-flex h-9 w-fit items-center gap-2 rounded-md border border-line px-3 text-fg transition-colors duration-150 hover:bg-hover disabled:opacity-50"
      >
        <FolderOpen size={16} strokeWidth={1.5} aria-hidden="true" />
        {busy ? "Analyse…" : "Choisir un dossier…"}
      </button>

      {info && !info.exists && (
        <p role="alert" className="text-xs text-bad">
          Ce dossier n'existe pas.
        </p>
      )}
      {notGit ? (
        <div role="alert" className="flex flex-col gap-2 rounded-md bg-wait/10 px-3 py-2.5 text-xs">
          <span className="flex items-center gap-2 text-wait">
            <TriangleAlert size={14} strokeWidth={1.5} aria-hidden="true" />
            Ce dossier n'est pas un dépôt Git
          </span>
          <span className="truncate font-mono text-subtle">{info.path}</span>
          <button
            type="button"
            onClick={() => {
              setAcceptPlain(true);
              onPick({ path: info.path, name: info.name });
            }}
            className="w-fit text-fg underline underline-offset-2"
          >
            L'utiliser comme dossier simple
          </button>
        </div>
      ) : (
        choice && (
          <div className="flex flex-col gap-1">
            <span className="truncate font-mono text-xs text-muted" title={choice.path}>
              {choice.path}
            </span>
            {choice.git && <GitLine branch={choice.git.branch} remote={choice.git.remote} />}
          </div>
        )
      )}
      {inspectError && (
        <p className="text-xs text-subtle">
          Informations Git indisponibles ({inspectError}). Le dossier sera lié tel quel.
        </p>
      )}
    </div>
  );
}

/** "Lier un workspace": pick a project from an open app, a Git repository or any folder, then name it. */
export function LinkWorkspaceDialog({ onClose }: { onClose: () => void }) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const tabRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const [source, setSource] = useState<Source>("apps");
  const [choice, setChoice] = useState<Choice | null>(null);
  const [name, setName] = useState("");

  useEffect(() => dialogRef.current?.showModal(), []);

  function pick(next: Choice | null) {
    setChoice(next);
    if (next) setName(next.name || folderName(next.path));
  }

  function switchSource(next: Source) {
    setSource(next);
    setChoice(null);
    setName("");
  }

  function onTabKey(e: KeyboardEvent, index: number) {
    const delta = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
    if (!delta) return;
    const next = (index + delta + SOURCES.length) % SOURCES.length;
    switchSource(SOURCES[next].id);
    tabRefs.current[next]?.focus();
  }

  function submit(e: FormEvent) {
    e.preventDefault();
    if (!choice) return;
    const existing = workspaceForPath(choice.path, useDeck.getState().workspaces);
    if (existing) {
      deck.selectWorkspace(existing.id);
      deck.toast(`« ${existing.name} » est déjà lié.`);
    } else {
      const git = choice.git ? { branch: choice.git.branch } : undefined;
      deck.linkWorkspace(choice.path, name.trim() || undefined, git);
    }
    onClose();
  }

  return (
    <dialog
      ref={dialogRef}
      onClose={onClose}
      aria-labelledby="link-ws-title"
      className="m-auto w-[min(34rem,92vw)] rounded-xl border border-line bg-panel p-0 text-fg backdrop:bg-gutter/70"
    >
      <form onSubmit={submit} className="flex flex-col gap-5 p-5">
        <div className="flex items-center justify-between">
          <h2 id="link-ws-title" className="text-[15px] font-medium">
            Lier un workspace
          </h2>
          <IconButton icon={X} label="Fermer" onClick={onClose} />
        </div>

        <div role="tablist" aria-label="Source" className="grid grid-cols-3 gap-1 rounded-lg bg-base p-1">
          {SOURCES.map(({ id, label, icon: Icon }, i) => (
            <button
              key={id}
              ref={(el) => {
                tabRefs.current[i] = el;
              }}
              type="button"
              role="tab"
              id={`link-ws-tab-${id}`}
              aria-selected={source === id}
              aria-controls="link-ws-panel"
              tabIndex={source === id ? 0 : -1}
              onClick={() => switchSource(id)}
              onKeyDown={(e) => onTabKey(e, i)}
              className={`inline-flex h-8 items-center justify-center gap-1.5 rounded-md text-xs transition-colors duration-150 ${
                source === id ? "bg-raised font-medium text-fg" : "text-muted hover:text-fg"
              }`}
            >
              <Icon size={14} strokeWidth={1.5} aria-hidden="true" />
              {label}
            </button>
          ))}
        </div>

        <div id="link-ws-panel" role="tabpanel" aria-labelledby={`link-ws-tab-${source}`} className="min-h-28">
          {source === "apps" && <OpenApps selected={choice?.path ?? null} onPick={pick} />}
          {source !== "apps" && (
            <FolderSource key={source} requireGit={source === "git"} choice={choice} onPick={pick} />
          )}
        </div>

        <div className="flex flex-col gap-1.5">
          <label htmlFor="link-ws-name" className="text-xs text-muted">
            Nom
          </label>
          <input
            id="link-ws-name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            disabled={!choice}
            placeholder={choice ? folderName(choice.path) : "Choisissez d'abord un projet"}
            className="h-9 rounded-md border border-line bg-base px-2.5 text-fg outline-none transition-colors duration-150 placeholder:text-subtle hover:border-line-strong focus:border-accent disabled:opacity-50"
          />
        </div>

        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="h-8 rounded-md px-3 text-muted transition-colors duration-150 hover:bg-hover hover:text-fg"
          >
            Annuler
          </button>
          <button
            type="submit"
            disabled={!choice}
            className="h-8 rounded-md bg-accent px-3.5 font-medium text-on-accent transition-colors duration-150 hover:bg-accent-hover disabled:opacity-40"
          >
            Lier le workspace
          </button>
        </div>
      </form>
    </dialog>
  );
}
