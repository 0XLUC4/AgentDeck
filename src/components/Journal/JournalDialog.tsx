import { ClipboardCopy, Download, GitCommitHorizontal, NotebookText } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { AGENTS } from "../../lib/agents";
import { journalGitCommits, journalRead, usageSummary, writeTextFile } from "../../lib/ipc";
import type { UsageSummary } from "../../lib/types";
import { deck, useDeck } from "../../store/deck";
import { AgentLogo } from "../AgentLogo/AgentLogo";
import { IconButton } from "../IconButton/IconButton";
import { formatCost, formatTokens } from "../Usage/format";
import { Modal } from "../Usage/Modal";
import {
  clockTime,
  dayKey,
  groupJournal,
  type JournalEntry,
  journalMarkdown,
  parseEntries,
  taskLine,
  type WorkspaceCommits,
} from "./journal";

interface Loaded {
  entries: JournalEntry[];
  commits: WorkspaceCommits[];
  usage: UsageSummary | null;
}

const warn = (what: string) => (err: unknown) => {
  console.warn(`${what} failed`, err);
  return null;
};

async function load(date: string, workspaces: { name: string; path: string }[]): Promise<Loaded> {
  const [raw, usage] = await Promise.all([journalRead(date), usageSummary("today").catch(warn("usage_summary"))]);
  const entries = parseEntries(raw);
  // Linked workspaces plus folders tasks ran in outside any workspace (deduplicated by path).
  const places = new Map<string, { name: string; path: string }>();
  for (const w of workspaces) if (w.path) places.set(w.path.toLowerCase(), w);
  for (const e of entries) {
    const path = e.workspacePath || e.cwd;
    if (path && !places.has(path.toLowerCase())) places.set(path.toLowerCase(), { name: e.workspaceName, path });
  }
  const commits = await Promise.all(
    [...places.values()].map(async (p) => ({
      ...p,
      commits: (await journalGitCommits(p.path).catch(warn("journal_git_commits"))) ?? [],
    })),
  );
  return { entries, commits, usage };
}

export function JournalDialog({ onClose }: { onClose: () => void }) {
  const date = useMemo(() => dayKey(), []);
  const [data, setData] = useState<Loaded | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    load(date, useDeck.getState().workspaces).then(
      (d) => alive && setData(d),
      (err) => alive && setError(String(err)),
    );
    return () => {
      alive = false;
    };
  }, [date]);

  const groups = useMemo(() => (data ? groupJournal(data.entries, data.commits) : []), [data]);
  const markdown = () => journalMarkdown(date, groups, data?.usage ?? null);

  const copy = () =>
    navigator.clipboard.writeText(markdown()).then(
      () => deck.toast("Journal copié en Markdown"),
      (err) => deck.toast(`Copie impossible : ${err}`),
    );
  const exportFile = () =>
    writeTextFile(`journal-${date}.md`, markdown()).then(
      (path) => path && deck.toast(`Journal exporté : ${path}`),
      (err) => deck.toast(`Export impossible : ${err}`),
    );

  const [y, m, d] = date.split("-");
  const usage = data?.usage;
  return (
    <Modal
      title={`Journal du ${d}/${m}/${y}`}
      onClose={onClose}
      actions={
        <>
          <IconButton icon={ClipboardCopy} label="Copier en Markdown" onClick={copy} disabled={!data} />
          <IconButton icon={Download} label="Exporter (.md)" onClick={exportFile} disabled={!data} />
        </>
      }
    >
      {error ? (
        <p role="alert" className="text-xs text-bad">
          Journal indisponible : {error}
        </p>
      ) : !data ? (
        <p className="py-8 text-center text-muted" aria-busy="true">
          Préparation du journal…
        </p>
      ) : (
        <div className="flex flex-col gap-6">
          {usage && usage.total.tokens > 0 && (
            <p className="text-muted">
              <span className="text-fg">{formatCost(usage.total.cost)}</span> · {formatTokens(usage.total.tokens)}{" "}
              tokens aujourd'hui (Claude Code, Codex)
            </p>
          )}
          {groups.length === 0 && <p className="py-6 text-center text-muted">Aucune tâche ni commit aujourd'hui</p>}
          {groups.map((g) => (
            <section key={g.key || g.name} className="flex flex-col gap-3">
              <div className="flex items-baseline gap-2 border-b border-line pb-1.5">
                <h3 className="text-[15px] font-medium">{g.name}</h3>
                {g.key && <span className="truncate font-mono text-xs text-subtle">{g.key}</span>}
              </div>
              {g.agents.map(({ agent, tasks }) => (
                <div key={agent} className="flex flex-col gap-1.5">
                  <h4 className="flex items-center gap-2 text-xs font-medium text-muted">
                    <AgentLogo kind={agent} size={13} />
                    {AGENTS[agent].name} · {tasks.length} tâche(s)
                  </h4>
                  <ul className="flex flex-col gap-2 pl-5">
                    {tasks.map((t) => (
                      <li key={`${t.paneId}-${t.startedAt}`} className="flex flex-col gap-0.5">
                        <span className="text-fg">{taskLine(t)}</span>
                        {t.summary && <span className="line-clamp-3 text-xs text-muted">{t.summary}</span>}
                        {t.actions.length > 0 && (
                          <span className="truncate text-xs text-subtle" title={t.actions.join("\n")}>
                            {t.actions.length} action(s) · {t.actions.slice(-3).join(" · ")}
                          </span>
                        )}
                      </li>
                    ))}
                  </ul>
                </div>
              ))}
              {g.commits.length > 0 && (
                <div className="flex flex-col gap-1.5">
                  <h4 className="flex items-center gap-2 text-xs font-medium text-muted">
                    <GitCommitHorizontal size={13} strokeWidth={1.5} aria-hidden="true" />
                    {g.commits.length} commit(s)
                  </h4>
                  <ul className="flex flex-col gap-1 pl-5">
                    {g.commits.map((c) => (
                      <li key={c.hash} className="flex gap-2 text-xs">
                        <span className="font-mono text-subtle">{c.hash}</span>
                        <span className="text-subtle tabular-nums">{clockTime(Date.parse(c.time))}</span>
                        <span className="truncate text-fg">{c.subject}</span>
                      </li>
                    ))}
                  </ul>
                </div>
              )}
            </section>
          ))}
        </div>
      )}
    </Modal>
  );
}

/** Title bar button + dialog. */
export function JournalButton() {
  const [open, setOpen] = useState(false);
  return (
    <>
      <IconButton icon={NotebookText} label="Journal du jour" onClick={() => setOpen(true)} />
      {open && <JournalDialog onClose={() => setOpen(false)} />}
    </>
  );
}
