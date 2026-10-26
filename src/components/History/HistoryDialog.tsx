import { ArrowLeft, ChevronDown, ChevronUp, History, Search, TriangleAlert } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { AGENTS } from "../../lib/agents";
import { historyRead, historySearch } from "../../lib/ipc";
import type { HistoryHit, HistoryRunMeta, HistoryWindow } from "../../lib/types";
import { folderName } from "../../store/deck";
import { AgentLogo } from "../AgentLogo/AgentLogo";
import { IconButton } from "../IconButton/IconButton";
import { isAgentKind } from "../Usage/format";
import { Modal } from "../Usage/Modal";

const LIMIT = 200;
const DEBOUNCE_MS = 250;

/** Splits `text` around case-insensitive occurrences of `query`, marking the matches. */
export function highlight(text: string, query: string): { text: string; match: boolean }[] {
  const q = query.toLowerCase();
  if (!q) return [{ text, match: false }];
  const lower = text.toLowerCase();
  const parts: { text: string; match: boolean }[] = [];
  let from = 0;
  for (let at = lower.indexOf(q); at !== -1; at = lower.indexOf(q, at + q.length)) {
    if (at > from) parts.push({ text: text.slice(from, at), match: false });
    parts.push({ text: text.slice(at, at + q.length), match: true });
    from = at + q.length;
  }
  if (from < text.length) parts.push({ text: text.slice(from), match: false });
  return parts;
}

function Highlighted({ text, query }: { text: string; query: string }) {
  return (
    <>
      {highlight(text, query).map((p, i) =>
        p.match ? (
          // biome-ignore lint/suspicious/noArrayIndexKey: static split of one string
          <mark key={i} className="rounded-sm bg-wait/30 text-fg">
            {p.text}
          </mark>
        ) : (
          // biome-ignore lint/suspicious/noArrayIndexKey: static split of one string
          <span key={i}>{p.text}</span>
        ),
      )}
    </>
  );
}

const when = (ms: number) =>
  new Date(ms).toLocaleString("fr-FR", { day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit" });

function RunHeader({ meta, runId }: { meta: HistoryRunMeta | null; runId: string }) {
  const kind = meta && isAgentKind(meta.agent) ? meta.agent : null;
  return (
    <div className="flex min-w-0 items-center gap-2">
      {kind && <AgentLogo kind={kind} size={14} />}
      <span className="truncate font-medium text-fg">{meta?.title || (kind ? AGENTS[kind].name : runId)}</span>
      {meta?.cwd && (
        <span className="truncate font-mono text-xs text-subtle" title={meta.cwd}>
          {folderName(meta.cwd)}
        </span>
      )}
      {meta && <span className="ml-auto shrink-0 text-xs text-subtle tabular-nums">{when(meta.startedAt)}</span>}
    </div>
  );
}

function groupByRun(hits: HistoryHit[]): [string, HistoryHit[]][] {
  const groups = new Map<string, HistoryHit[]>();
  for (const h of hits) groups.set(h.runId, [...(groups.get(h.runId) ?? []), h]);
  return [...groups];
}

function Viewer({ runId, line, query, onBack }: { runId: string; line: number; query: string; onBack: () => void }) {
  const [around, setAround] = useState(line);
  const [win, setWin] = useState<HistoryWindow | null>(null);
  const [error, setError] = useState<string | null>(null);
  const targetRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let alive = true;
    historyRead(runId, around).then(
      (w) => alive && setWin(w),
      (err) => alive && setError(String(err)),
    );
    return () => {
      alive = false;
    };
  }, [runId, around]);

  useLayoutEffect(() => {
    if (win) targetRef.current?.scrollIntoView({ block: "center" });
  }, [win]);

  const end = win ? win.startLine + win.lines.length : 0;
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2">
        <IconButton icon={ArrowLeft} label="Retour aux résultats" onClick={onBack} />
        <div className="min-w-0 flex-1">{win && <RunHeader meta={win.meta} runId={runId} />}</div>
      </div>
      {error && <p className="text-xs text-bad">{error}</p>}
      {win && (
        <>
          {win.startLine > 0 && (
            <button
              type="button"
              onClick={() => setAround(Math.max(0, win.startLine - 1000))}
              className="inline-flex items-center gap-1.5 self-center text-xs text-muted hover:text-fg"
            >
              <ChevronUp size={14} strokeWidth={1.5} aria-hidden="true" />
              Lignes précédentes
            </button>
          )}
          <div className="overflow-x-auto rounded-md bg-base py-2 font-mono text-xs leading-5">
            {win.lines.map((text, i) => {
              const n = win.startLine + i;
              const target = n === line;
              return (
                <div
                  key={n}
                  ref={target ? targetRef : undefined}
                  className={`flex gap-3 px-2 ${target ? "bg-raised" : ""}`}
                >
                  <span className="w-12 shrink-0 select-none text-right text-subtle tabular-nums">{n + 1}</span>
                  <span className="whitespace-pre text-muted">
                    <Highlighted text={text} query={query} />
                  </span>
                </div>
              );
            })}
          </div>
          {end < win.totalLines && (
            <button
              type="button"
              onClick={() => setAround(end + 1000)}
              className="inline-flex items-center gap-1.5 self-center text-xs text-muted hover:text-fg"
            >
              <ChevronDown size={14} strokeWidth={1.5} aria-hidden="true" />
              Lignes suivantes
            </button>
          )}
        </>
      )}
    </div>
  );
}

export function HistoryDialog({ onClose }: { onClose: () => void }) {
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<HistoryHit[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState<{ runId: string; line: number } | null>(null);

  useEffect(() => {
    const q = query.trim();
    if (!q) {
      setHits(null);
      return;
    }
    let alive = true;
    const timer = setTimeout(() => {
      historySearch(q, LIMIT).then(
        (h) => {
          if (!alive) return;
          setHits(h);
          setError(null);
        },
        (err) => alive && setError(String(err)),
      );
    }, DEBOUNCE_MS);
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [query]);

  return (
    <Modal title="Historique des terminaux" onClose={onClose} className="w-[min(56rem,94vw)]">
      {open ? (
        <Viewer runId={open.runId} line={open.line} query={query.trim()} onBack={() => setOpen(null)} />
      ) : (
        <div className="flex flex-col gap-4">
          <label className="flex h-9 items-center gap-2 rounded-md border border-line bg-base px-2.5 focus-within:border-accent">
            <Search size={15} strokeWidth={1.5} className="shrink-0 text-subtle" aria-hidden="true" />
            <input
              // biome-ignore lint/a11y/noAutofocus: the dialog exists to type a search
              autoFocus
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Rechercher dans la sortie de tous les terminaux…"
              aria-label="Rechercher"
              className="min-w-0 flex-1 bg-transparent text-fg outline-none placeholder:text-subtle"
            />
          </label>
          {error && (
            <p role="alert" className="flex items-center gap-2 text-xs text-bad">
              <TriangleAlert size={14} strokeWidth={1.5} aria-hidden="true" />
              {error}
            </p>
          )}
          {hits === null ? (
            <p className="py-8 text-center text-muted">
              Sortie des terminaux enregistrée en texte brut, conservée 30 jours.
            </p>
          ) : hits.length === 0 ? (
            <p className="py-8 text-center text-muted">Aucun résultat</p>
          ) : (
            <>
              <p className="text-xs text-subtle" aria-live="polite">
                {hits.length >= LIMIT ? `${LIMIT}+ résultats (les plus récents)` : `${hits.length} résultat(s)`}
              </p>
              {groupByRun(hits).map(([runId, group]) => (
                <section key={runId} className="flex flex-col gap-1.5">
                  <RunHeader meta={group[0].meta} runId={runId} />
                  <ul className="flex flex-col">
                    {group.map((hit) => (
                      <li key={hit.line}>
                        <button
                          type="button"
                          onClick={() => setOpen({ runId, line: hit.line })}
                          className="flex w-full gap-3 rounded-md px-2 py-1 text-left font-mono text-xs leading-5 transition-colors duration-150 hover:bg-hover"
                        >
                          <span className="w-12 shrink-0 text-right text-subtle tabular-nums">{hit.line + 1}</span>
                          <span className="flex min-w-0 flex-col">
                            {hit.before && <span className="truncate text-subtle">{hit.before}</span>}
                            <span className="truncate text-muted">
                              <Highlighted text={hit.text} query={query.trim()} />
                            </span>
                            {hit.after && <span className="truncate text-subtle">{hit.after}</span>}
                          </span>
                        </button>
                      </li>
                    ))}
                  </ul>
                </section>
              ))}
            </>
          )}
        </div>
      )}
    </Modal>
  );
}

/** Title bar button + Ctrl+Shift+F. */
export function HistoryButton() {
  const [open, setOpen] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey && e.shiftKey && !e.altKey && e.code === "KeyF")) return;
      e.preventDefault();
      e.stopPropagation(); // capture phase: the terminal never sees it
      if (!e.repeat) setOpen(true);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);

  return (
    <>
      <IconButton
        icon={History}
        label="Historique"
        hint="Rechercher dans l'historique (Ctrl+Maj+F)"
        onClick={() => setOpen(true)}
      />
      {open && <HistoryDialog onClose={() => setOpen(false)} />}
    </>
  );
}
