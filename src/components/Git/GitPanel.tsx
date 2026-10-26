import { GitBranch, GitCommitHorizontal, GitMerge, History, RefreshCw, Trash2, X } from "lucide-react";
import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AGENTS } from "../../lib/agents";
import { checkpointPane, paneGitDir } from "../../lib/checkpoints";
import {
  gitCommit,
  gitCompare,
  gitDiff,
  gitDiffTrees,
  gitListCheckpoints,
  gitMergeWorktree,
  gitRemoveWorktree,
  gitRestoreCheckpoint,
  gitStatus,
} from "../../lib/ipc";
import type { GitCheckpoint, GitComparison, GitFileChange, GitStatus } from "../../lib/types";
import { deck, type Pane, useDeck } from "../../store/deck";
import { IconButton } from "../IconButton/IconButton";
import { classifyDiff, type DiffLineKind, diffLines } from "./diff";

const BTN =
  "inline-flex h-7 items-center justify-center gap-1.5 rounded-md border border-line-strong px-2.5 text-xs font-medium text-fg transition-colors duration-150 hover:bg-hover disabled:pointer-events-none disabled:opacity-35";
const BTN_DANGER = `${BTN} hover:bg-bad/15 hover:text-bad`;
const DIFF_PAGE = 500;
const LINE_CLASS: Record<DiffLineKind, string> = {
  add: "bg-ok/10 text-ok",
  del: "bg-bad/10 text-bad",
  hunk: "text-subtle",
  meta: "text-subtle",
  context: "text-muted",
};

const paneName = (pane: Pane) => pane.title || AGENTS[pane.agent].name;
const errorText = (err: unknown) => (err instanceof Error ? err.message : String(err));
// Keeps the PaneHeader (our React parent through the portal) from seeing clicks: no focus steal, no maximize.
const stop = (e: { stopPropagation: () => void }) => e.stopPropagation();

function DiffView({ text }: { text: string }) {
  const [shown, setShown] = useState(DIFF_PAGE);
  const lines = useMemo(() => diffLines(text), [text]);
  const kinds = useMemo(() => classifyDiff(lines), [lines]);
  if (lines.length === 0) return <p className="p-4 text-xs text-subtle">Aucune différence.</p>;
  return (
    <div className="min-w-max py-2 font-mono text-xs leading-5">
      {lines.slice(0, shown).map((line, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: static list of lines, never reordered
        <div key={i} className={`whitespace-pre px-3 ${LINE_CLASS[kinds[i]]}`}>
          {line || " "}
        </div>
      ))}
      {lines.length > shown && (
        <button type="button" className={`${BTN} m-3`} onClick={() => setShown((n) => n + DIFF_PAGE * 4)}>
          Afficher plus ({lines.length - shown} lignes restantes)
        </button>
      )}
    </div>
  );
}

function FileRow({ file, selected, onSelect }: { file: GitFileChange; selected: boolean; onSelect: () => void }) {
  return (
    <li>
      <button
        type="button"
        aria-current={selected ? "true" : undefined}
        title={file.path}
        onClick={onSelect}
        className={`flex w-full items-center gap-2 px-3 py-1 text-left text-xs transition-colors duration-150 ${
          selected ? "bg-raised text-fg" : "text-muted hover:bg-hover"
        }`}
      >
        <span className="w-3 shrink-0 font-mono text-subtle">{file.status}</span>
        <span className="min-w-0 flex-1 truncate">{file.path}</span>
        {file.additions === null ? (
          <span className="shrink-0 text-subtle">bin</span>
        ) : (
          <span className="shrink-0 font-mono tabular-nums">
            <span className="text-ok">+{file.additions}</span> <span className="text-bad">−{file.deletions}</span>
          </span>
        )}
      </button>
    </li>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2 border-t border-line px-3 py-3" aria-label={title}>
      <h3 className="text-xs font-medium text-subtle">{title}</h3>
      {children}
    </section>
  );
}

/** Right slide-over with the Git state of one pane's working directory (its worktree when it has one). */
export function GitPanel({ pane, onClose }: { pane: Pane; onClose: () => void }) {
  const panes = useDeck((s) => s.panes);
  const [dir, setDir] = useState<string | null>(null);
  const [status, setStatus] = useState<GitStatus | null>(null);
  const [checkpoints, setCheckpoints] = useState<GitCheckpoint[]>([]);
  const [comparison, setComparison] = useState<(GitComparison & { otherId: string }) | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [diff, setDiff] = useState<string | null>(null);
  const [notice, setNotice] = useState<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const panelRef = useRef<HTMLElement>(null);
  const selectedRef = useRef<string | null>(null);

  useEffect(() => panelRef.current?.focus(), []);

  const { id, cwd, worktree } = pane;
  useEffect(() => {
    paneGitDir({ id, cwd, worktree }).then(setDir, (err) => setNotice({ ok: false, text: errorText(err) }));
  }, [id, cwd, worktree]);

  const refresh = useCallback(async () => {
    if (!dir) return;
    const [st, cps] = await Promise.all([gitStatus(dir), gitListCheckpoints(dir, pane.id)]);
    setStatus(st);
    setCheckpoints(cps);
  }, [dir, pane.id]);

  useEffect(() => {
    refresh().catch((err) => setNotice({ ok: false, text: errorText(err) }));
  }, [refresh]);

  const openFile = useCallback(
    async (path: string | null, cmp = comparison) => {
      selectedRef.current = path;
      setSelected(path);
      setDiff(null);
      if (!path || !dir) return;
      try {
        const text = cmp ? await gitDiffTrees(dir, cmp.from, cmp.to, path) : await gitDiff(dir, path);
        if (selectedRef.current === path) setDiff(text);
      } catch (err) {
        setNotice({ ok: false, text: errorText(err) });
      }
    },
    [dir, comparison],
  );

  /** Runs one action with a busy state; reports its result and refreshes. */
  const act = async (job: () => Promise<string | null | undefined>) => {
    setBusy(true);
    setNotice(null);
    try {
      const done = await job();
      if (done) setNotice({ ok: true, text: done });
      await refresh();
      if (selectedRef.current && !comparison) await openFile(selectedRef.current);
    } catch (err) {
      setNotice({ ok: false, text: errorText(err) });
    } finally {
      setBusy(false);
    }
  };

  const compareWith = (otherId: string) => {
    const other = panes[otherId];
    if (!other || !dir) {
      setComparison(null);
      void openFile(null, null);
      return;
    }
    void act(async () => {
      const cmp = { ...(await gitCompare(dir, await paneGitDir(other))), otherId };
      setComparison(cmp);
      await openFile(null, cmp);
    });
  };

  const commit = () =>
    act(async () => {
      if (!dir) return;
      const sha = await gitCommit(dir, message);
      setMessage("");
      return `Commit ${sha} créé.`;
    });

  const merge = () => {
    if (!dir || !window.confirm(`Merger ${status?.branch} dans le dépôt principal (--no-ff) ?`)) return;
    void act(() => gitMergeWorktree(dir));
  };

  const removeWorktree = () => {
    if (!dir) return;
    const ok = window.confirm(
      `Supprimer le worktree ${dir} et sa branche ${status?.branch} ? Les commits non mergés seront perdus.`,
    );
    if (!ok) return;
    setBusy(true);
    gitRemoveWorktree(dir).then(
      () => {
        deck.toast(`Worktree ${status?.branch} supprimé.`);
        onClose();
      },
      (err) => {
        setNotice({ ok: false, text: errorText(err) });
        setBusy(false);
      },
    );
  };

  const restore = (cp: GitCheckpoint) => {
    const when = new Date(cp.ts).toLocaleString("fr-FR");
    const running = pane.exited
      ? ""
      : "\n\nL'agent tourne encore : arrêtez-le d'abord pour éviter qu'il réécrive des fichiers.";
    const ok = window.confirm(
      `Restaurer les fichiers tels qu'au ${when} ? Les modifications faites depuis (et les commits) seront annulées. L'état actuel est d'abord sauvegardé dans un nouveau point de restauration.${running}`,
    );
    if (!ok || !dir) return;
    void act(async () => {
      await gitRestoreCheckpoint(dir, pane.id, cp.sha);
      return `Fichiers restaurés au ${when}.`;
    });
  };

  const files = comparison?.files ?? status?.files ?? [];
  const others = Object.values(panes).filter((p) => p.id !== pane.id);
  const base = status?.base ?? "la branche principale";

  return createPortal(
    // biome-ignore lint/a11y/noStaticElementInteractions: only stops event bubbling to the pane header
    <div
      className="fixed inset-0 z-40 flex justify-end"
      onMouseDown={stop}
      onPointerDown={stop}
      onClick={stop}
      onDoubleClick={stop}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Escape") onClose();
      }}
    >
      <button
        type="button"
        tabIndex={-1}
        aria-label="Fermer le panneau Git"
        className="absolute inset-0 cursor-default bg-black/40"
        onClick={onClose}
      />
      <aside
        ref={panelRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-label={`Git : ${paneName(pane)}`}
        className="relative flex h-full w-[min(960px,92vw)] flex-col border-l border-line-strong bg-panel text-[13px] text-fg outline-none"
      >
        <header className="flex h-10 shrink-0 items-center gap-2 border-b border-line pr-2 pl-3">
          <GitBranch size={15} strokeWidth={1.5} className="text-subtle" aria-hidden="true" />
          <h2 className="font-medium">Git</h2>
          <span className="min-w-0 flex-1 truncate text-xs text-subtle">{paneName(pane)}</span>
          <IconButton
            icon={RefreshCw}
            label="Rafraîchir"
            disabled={busy || !dir}
            onClick={() => {
              setComparison(null);
              void act(async () => {
                await openFile(null, null);
                return null;
              });
            }}
          />
          <IconButton icon={X} label="Fermer" hint="Fermer (Échap)" onClick={onClose} />
        </header>

        <div className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1 border-b border-line px-3 py-2 text-xs">
          <span className="font-mono text-fg">{status ? (status.branch ?? "HEAD détachée") : "…"}</span>
          {status?.base && status.branch !== status.base && (
            <span className="tabular-nums text-muted" title={`Commits en avance / en retard sur ${status.base}`}>
              ↑{status.ahead} ↓{status.behind} vs {status.base}
            </span>
          )}
          <span className="min-w-0 flex-1 truncate text-right font-mono text-subtle" title={dir ?? undefined}>
            {dir}
          </span>
        </div>

        {notice && (
          <p
            role={notice.ok ? "status" : "alert"}
            className={`shrink-0 border-b border-line px-3 py-2 text-xs whitespace-pre-wrap ${
              notice.ok ? "text-muted" : "bg-bad/10 text-bad"
            }`}
          >
            {notice.text}
          </p>
        )}

        <div className="flex min-h-0 flex-1">
          <div className="scroll-thin flex w-80 shrink-0 flex-col overflow-y-auto border-r border-line">
            <label className="flex flex-col gap-1 px-3 py-2 text-xs text-subtle">
              Comparer avec…
              <select
                className="h-7 rounded-md border border-line-strong bg-base px-1.5 text-fg outline-none focus:border-accent"
                value={comparison?.otherId ?? ""}
                disabled={busy || !dir}
                onChange={(e) => compareWith(e.target.value)}
              >
                <option value="">Modifications en cours (vs HEAD)</option>
                {others.map((p) => (
                  <option key={p.id} value={p.id}>
                    {paneName(p)} · {AGENTS[p.agent].name}
                  </option>
                ))}
              </select>
            </label>

            <h3 className="flex items-center px-3 pt-1 pb-1 text-xs text-subtle">
              {comparison
                ? `Différences avec ${panes[comparison.otherId] ? paneName(panes[comparison.otherId]) : "…"}`
                : "Fichiers modifiés"}
              <span className="ml-auto tabular-nums">{files.length}</span>
            </h3>
            {files.length === 0 ? (
              <p className="px-3 pb-3 text-xs text-subtle">{status ? "Aucune modification." : "Chargement…"}</p>
            ) : (
              <ul className="pb-2">
                {files.map((f) => (
                  <FileRow
                    key={f.path}
                    file={f}
                    selected={f.path === selected}
                    onSelect={() => void openFile(f.path)}
                  />
                ))}
              </ul>
            )}

            {!comparison && (
              <Section title="Commit">
                <textarea
                  rows={2}
                  value={message}
                  onChange={(e) => setMessage(e.target.value)}
                  placeholder="feat(scope): description"
                  aria-label="Message de commit"
                  className="resize-y rounded-md border border-line-strong bg-base px-2 py-1.5 text-xs text-fg outline-none placeholder:text-subtle focus:border-accent"
                />
                <p className="text-xs text-subtle">
                  Conventional Commits : feat, fix, refactor, docs, test, chore. Tout est indexé (git add -A).
                </p>
                <button
                  type="button"
                  className={BTN}
                  disabled={busy || !message.trim() || !status?.files.length}
                  onClick={() => void commit()}
                >
                  <GitCommitHorizontal size={14} strokeWidth={1.5} aria-hidden="true" />
                  Commit
                </button>
              </Section>
            )}

            {status?.agentWorktree && (
              <Section title="Worktree">
                <button type="button" className={BTN} disabled={busy} onClick={merge}>
                  <GitMerge size={14} strokeWidth={1.5} aria-hidden="true" />
                  Merge vers {base}
                </button>
                <button
                  type="button"
                  className={BTN_DANGER}
                  disabled={busy || !pane.exited}
                  title={pane.exited ? undefined : "Arrêtez l'agent avant de supprimer son worktree"}
                  onClick={removeWorktree}
                >
                  <Trash2 size={14} strokeWidth={1.5} aria-hidden="true" />
                  Supprimer le worktree
                </button>
              </Section>
            )}

            <Section title="Points de restauration">
              <button
                type="button"
                className={BTN}
                disabled={busy || !dir}
                onClick={() =>
                  void act(async () => ((await checkpointPane(pane.id)) ? "Point de restauration créé." : null))
                }
              >
                <History size={14} strokeWidth={1.5} aria-hidden="true" />
                Créer un point de restauration
              </button>
              {checkpoints.length === 0 ? (
                <p className="text-xs text-subtle">Aucun point pour cet agent.</p>
              ) : (
                <ul className="flex flex-col">
                  {checkpoints.map((cp) => (
                    <li key={cp.ts} className="flex items-center gap-2 py-0.5 text-xs">
                      <span className="tabular-nums text-muted">{new Date(cp.ts).toLocaleString("fr-FR")}</span>
                      <span className="font-mono text-subtle">{cp.sha.slice(0, 7)}</span>
                      <button
                        type="button"
                        className="ml-auto rounded-md px-1.5 py-0.5 text-fg transition-colors hover:bg-hover disabled:opacity-35"
                        disabled={busy}
                        onClick={() => restore(cp)}
                      >
                        Restaurer
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </Section>
          </div>

          <div className="scroll-thin min-w-0 flex-1 overflow-auto bg-base">
            {selected === null ? (
              <p className="p-4 text-xs text-subtle">Sélectionnez un fichier pour voir son diff.</p>
            ) : diff === null ? (
              <p className="p-4 text-xs text-subtle">Chargement du diff…</p>
            ) : (
              <DiffView key={selected} text={diff} />
            )}
          </div>
        </div>
      </aside>
    </div>,
    document.body,
  );
}
