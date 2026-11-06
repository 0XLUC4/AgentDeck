import { useEffect, useState } from "react";
import { gitPaneDir, gitStatus } from "../../lib/ipc";
import type { Pane } from "../../store/deck";

interface Diff {
  files: number;
  additions: number;
  deletions: number;
}

/** One git status costs about ten git processes: refresh once things settle, not on every tool call. */
const REFRESH_DELAY_MS = 2500;

/** "+12 -3 · 4" in the pane header: uncommitted changes in the agent's folder. Hidden when clean or not a repo. */
export function DiffChip({ pane, onOpen }: { pane: Pane; onOpen: () => void }) {
  const [diff, setDiff] = useState<Diff | null>(null);
  const { id, cwd, worktree, exited } = pane;

  // biome-ignore lint/correctness/useExhaustiveDependencies: status/completedCount only trigger the refresh
  useEffect(() => {
    if (exited) return;
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const status = await gitStatus(await gitPaneDir(cwd, id, worktree));
        if (cancelled) return;
        setDiff({
          files: status.files.length,
          additions: status.files.reduce((n, f) => n + (f.additions ?? 0), 0),
          deletions: status.files.reduce((n, f) => n + (f.deletions ?? 0), 0),
        });
      } catch {
        if (!cancelled) setDiff(null); // not a git repo
      }
    }, REFRESH_DELAY_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [id, cwd, worktree, exited, pane.status, pane.completedCount]);

  if (!diff || diff.files === 0) return null;
  const label = `${diff.files} changed file${diff.files > 1 ? "s" : ""}, ${diff.additions} added and ${diff.deletions} removed lines`;
  return (
    <button
      type="button"
      onClick={onOpen}
      title={`${label}. Open the Git panel`}
      aria-label={label}
      className="flex shrink-0 items-center gap-1 rounded px-1 font-mono text-[11px] tabular-nums text-subtle transition-colors hover:bg-hover hover:text-fg"
    >
      <span className="text-ok">+{diff.additions}</span>
      <span className="text-bad">-{diff.deletions}</span>
      <span>· {diff.files}</span>
    </button>
  );
}
