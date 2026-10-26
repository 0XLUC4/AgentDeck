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
    expect(routeTranscript("Codex, run the tests.", panes)).toEqual({ paneId: "x1", text: "run the tests." });
    expect(routeTranscript("Gemini: nothing", panes)).toEqual({ paneId: null, text: "Gemini: nothing" });
    expect(routeTranscript("Shell dir", panes)).toEqual({ paneId: "s1", text: "dir" });
  });

  it("is case, accent and mis-transcription tolerant", () => {
    expect(routeTranscript("CLAUDE do a review", panes).paneId).toBe("c1");
    expect(routeTranscript("Cloud, do a review", panes)).toEqual({ paneId: "c1", text: "do a review" });
    expect(routeTranscript("Codecs : fix the build", panes)).toEqual({ paneId: "x1", text: "fix the build" });
    expect(routeTranscript("Clàude Code, go", panes)).toEqual({ paneId: "c1", text: "go" });
  });

  it("prefers the first candidate (most recently active) for an agent kind", () => {
    const reordered = [panes[2], ...panes.filter((p) => p.id !== "c2")];
    expect(routeTranscript("Claude, go", reordered).paneId).toBe("c2");
  });

  it("matches pane titles, longest first", () => {
    expect(routeTranscript("Back end, add a route", panes)).toEqual({ paneId: "c2", text: "add a route" });
    expect(routeTranscript("front: the button", panes)).toEqual({ paneId: "c1", text: "the button" });
  });

  it("leaves the transcript alone when nothing matches", () => {
    expect(routeTranscript("Add a test", panes)).toEqual({ paneId: null, text: "Add a test" });
    expect(routeTranscript("Claude.", panes)).toEqual({ paneId: null, text: "Claude." });
    expect(routeTranscript("Codex", [])).toEqual({ paneId: null, text: "Codex" });
    // A name later in the sentence is not a prefix.
    expect(routeTranscript("Ask Claude", panes).paneId).toBeNull();
    expect(routeTranscript("Claudette is coming", panes).paneId).toBeNull();
  });
});
