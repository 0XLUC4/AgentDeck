// Writing a prompt into an agent's terminal and submitting it.
import { writeToPane } from "../../lib/ipc";

/** Agent TUIs read text + Enter arriving in one write as a paste and insert a newline instead of submitting. */
const SUBMIT_DELAY_MS = 120;

/** Text as the agent should receive it: multiline text goes as one bracketed paste. */
export function promptText(text: string): string {
  const body = text.replace(/\r\n?/g, "\n");
  return body.includes("\n") ? `\x1b[200~${body}\x1b[201~` : body;
}

/** Types the prompt, then presses Enter in a separate write when `submit`. */
export async function sendPrompt(paneId: string, text: string, submit = true): Promise<void> {
  await writeToPane(paneId, promptText(text));
  if (!submit) return;
  await new Promise((resolve) => setTimeout(resolve, SUBMIT_DELAY_MS));
  await writeToPane(paneId, "\r");
}
