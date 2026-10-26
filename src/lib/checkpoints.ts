// Task checkpoints: an exact snapshot of a pane's working tree (untracked files included) taken before
// each agent task, so the task can be undone from the Git panel. Never touches the working tree or index.
import type { Pane } from "../store/deck";
import { useDeck } from "../store/deck";
import { gitCheckpoint, gitPaneDir } from "./ipc";

/** Directory where the pane's agent actually runs (its git worktree when it has one). */
export const paneGitDir = (pane: Pick<Pane, "id" | "cwd" | "worktree">) => gitPaneDir(pane.cwd, pane.id, pane.worktree);

/** Records a checkpoint of the pane's working tree. Resolves to its sha, or null when not in a git repo. */
export async function checkpointPane(paneId: string): Promise<string | null> {
  const pane = useDeck.getState().panes[paneId];
  if (!pane) return null;
  return gitCheckpoint(await paneGitDir(pane), paneId);
}

/** To call when an agent starts a task (fire and forget): takes the automatic checkpoint. */
export function onTaskStart(paneId: string): void {
  checkpointPane(paneId).catch((err) => console.warn("checkpoint failed", err));
}
