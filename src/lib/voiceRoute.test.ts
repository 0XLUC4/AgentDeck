import { describe, expect, it } from "vitest";
import { type RouteCandidate, routeTranscript } from "./voiceRoute";

const panes: RouteCandidate[] = [
  { id: "c1", agent: "claude", title: "Front" },
  { id: "x1", agent: "codex", title: "agentdeck" },
  { id: "c2", agent: "claude", title: "Back end" },
  { id: "s1", agent: "shell", title: "Shell" },
];

describe("routeTranscript", () => {
  it("routes an agent name prefix and strips it", () => {
    expect(routeTranscript("Codex, lance les tests.", panes)).toEqual({ paneId: "x1", text: "lance les tests." });
    expect(routeTranscript("Gemini: rien", panes)).toEqual({ paneId: null, text: "Gemini: rien" });
    expect(routeTranscript("Shell dir", panes)).toEqual({ paneId: "s1", text: "dir" });
  });

  it("is case, accent and mis-transcription tolerant", () => {
    expect(routeTranscript("CLAUDE fais une revue", panes).paneId).toBe("c1");
    expect(routeTranscript("Cloud, fais une revue", panes)).toEqual({ paneId: "c1", text: "fais une revue" });
    expect(routeTranscript("Codecs : corrige le build", panes)).toEqual({ paneId: "x1", text: "corrige le build" });
    expect(routeTranscript("Clàude Code, go", panes)).toEqual({ paneId: "c1", text: "go" });
  });

  it("prefers the first candidate (most recently active) for an agent kind", () => {
    const reordered = [panes[2], ...panes.filter((p) => p.id !== "c2")];
    expect(routeTranscript("Claude, go", reordered).paneId).toBe("c2");
  });

  it("matches pane titles, longest first", () => {
    expect(routeTranscript("Back end, ajoute une route", panes)).toEqual({ paneId: "c2", text: "ajoute une route" });
    expect(routeTranscript("front: le bouton", panes)).toEqual({ paneId: "c1", text: "le bouton" });
  });

  it("leaves the transcript alone when nothing matches", () => {
    expect(routeTranscript("Ajoute un test", panes)).toEqual({ paneId: null, text: "Ajoute un test" });
    expect(routeTranscript("Claude.", panes)).toEqual({ paneId: null, text: "Claude." });
    expect(routeTranscript("Codex", [])).toEqual({ paneId: null, text: "Codex" });
    // A name later in the sentence is not a prefix.
    expect(routeTranscript("Demande à Claude", panes).paneId).toBeNull();
    expect(routeTranscript("Claudette arrive", panes).paneId).toBeNull();
  });
});
