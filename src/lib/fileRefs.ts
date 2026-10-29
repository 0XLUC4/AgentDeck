// Turning pasted/dropped files into text an agent's prompt understands (it attaches images from their path).
import type { AgentKind } from "./types";

/** Paths as the agent expects them, space-separated with a trailing space: Gemini wants `@path`, others a (quoted) path. */
export function fileRefs(agent: AgentKind, paths: string[]): string {
  const refs = paths.map((p) => {
    if (agent === "gemini") return `@${p.replace(/ /g, "\\ ")}`;
    return /\s/.test(p) ? `"${p}"` : p;
  });
  return refs.length ? `${refs.join(" ")} ` : "";
}

const IMAGE_EXT: Record<string, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/gif": "gif",
  "image/webp": "webp",
  "image/bmp": "bmp",
};

/** File extension for a clipboard image, or undefined when it is not an image we save. */
export const imageExt = (mime: string): string | undefined => IMAGE_EXT[mime];
