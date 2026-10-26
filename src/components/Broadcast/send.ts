// Writing a prompt into an agent's terminal and submitting it.
import { writeToPane } from "../../lib/ipc";

/** Bytes that make the agent receive `text` as one prompt and submit it. Multiline text goes as a bracketed paste. */
export function promptPayload(text: string): string {
  const body = text.replace(/\r\n?/g, "\n");
  return body.includes("\n") ? `\x1b[200~${body}\x1b[201~\r` : `${body}\r`;
}

export const sendPrompt = (paneId: string, text: string) => writeToPane(paneId, promptPayload(text));
