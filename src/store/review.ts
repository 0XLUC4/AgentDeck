// Code then review: when an agent finishes a task, another agent is asked to review its changes.
// In memory only, like queues. The reviewer reads the diff itself (git), so nothing large goes through the PTY.
import { create } from "zustand";
import { sendPrompt } from "../components/Broadcast/send";
import { AGENTS } from "../lib/agents";
import { paneGitDir } from "../lib/checkpoints";
import { deck, type Pane, useDeck } from "./deck";
import { ADVANCE_DELAY_MS, queues, readyForNext, useQueues } from "./queue";
import { useTasks } from "./tasks";

/** author paneId -> reviewer paneId. */
export const useReviews = create<Record<string, string>>(() => ({}));

/** True when making `reviewerId` review `authorId` would close a loop (A reviews B reviews A...). */
export function createsLoop(map: Record<string, string>, authorId: string, reviewerId: string): boolean {
  const seen = new Set([authorId]);
  for (let id: string | undefined = reviewerId; id; id = map[id]) {
    if (seen.has(id)) return true;
    seen.add(id);
  }
  return false;
}

export function setReviewer(authorId: string, reviewerId: string | null): void {
  const { [authorId]: _old, ...rest } = useReviews.getState();
  if (!reviewerId) {
    useReviews.setState(rest, true);
    return;
  }
  if (createsLoop(rest, authorId, reviewerId)) {
    deck.toast("That reviewer already sends its work back to this agent: pick another one.");
    return;
  }
  useReviews.setState({ ...rest, [authorId]: reviewerId }, true);
}

export function reviewPrompt(author: Pick<Pane, "title" | "agent">, dir: string, task: string | null): string {
  const name = `${author.title} (${AGENTS[author.agent].name})`;
  return [
    `Review the work ${name} just finished in ${dir}.`,
    task ? `Its task was: "${task.replace(/\s+/g, " ").slice(0, 500)}".` : "",
    `See its changes with \`git -C "${dir}" status\` and \`git -C "${dir}" diff HEAD\` (untracked files too).`,
    "Report bugs, risks and missing tests, most serious first. Do not edit those files.",
  ]
    .filter(Boolean)
    .join(" ");
}

async function requestReview(authorId: string, reviewerId: string): Promise<void> {
  const { panes } = useDeck.getState();
  const author = panes[authorId];
  const reviewer = panes[reviewerId];
  if (!author || !reviewer || reviewer.exited) return;
  const task = useTasks.getState()[authorId]?.history[0];
  const text = reviewPrompt(author, await paneGitDir(author), task?.prompt ?? null);
  const idle = readyForNext(useDeck.getState().panes[reviewerId]) && !useQueues.getState()[reviewerId]?.items.length;
  if (idle) await sendPrompt(reviewerId, text);
  else queues.add(reviewerId, text); // sent when the reviewer finishes its current task
  deck.toast(`${reviewer.title} is reviewing ${author.title}'s changes.`);
}

/** Subscribes once (module-level call from App). */
export function startReviewRunner(): () => void {
  return useDeck.subscribe((state, prev) => {
    if (state.panes === prev.panes) return;
    for (const [authorId, reviewerId] of Object.entries(useReviews.getState())) {
      if (!state.panes[authorId] || !state.panes[reviewerId]) {
        setReviewer(authorId, null);
        continue;
      }
      if (prev.panes[authorId]?.status === "done" || state.panes[authorId].status !== "done") continue;
      setTimeout(() => {
        requestReview(authorId, reviewerId).catch((err) =>
          deck.toast(`Review not sent: ${err instanceof Error ? err.message : String(err)}`),
        );
      }, ADVANCE_DELAY_MS);
    }
  });
}
