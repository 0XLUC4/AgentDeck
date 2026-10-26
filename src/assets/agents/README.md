# Agent logos

The agent marks are no longer files here. They come from the
[`simple-icons`](https://simpleicons.org) npm package (CC0 for the SVG data) and are rendered
inline by `src/components/AgentLogo/AgentLogo.tsx`:

| Agent       | simple-icons slug | Color used                      |
| ----------- | ----------------- | ------------------------------- |
| Claude Code | `claude`          | `#D97757` (brand)               |
| Codex       | none (see below)  | lucide `Hexagon`, light neutral |
| Gemini CLI  | `googlegemini`    | `#8E75B2` (brand)               |
| Shell       | none              | lucide `SquareTerminal` icon    |

## Codex / OpenAI

simple-icons (v16) does not ship an OpenAI mark, so Codex uses a neutral lucide glyph. To use the
official mark, take the SVG from OpenAI's brand page, keep its path as-is, and set
`AGENTS.codex.mark` in `src/lib/agents.ts` to that path string (24x24 viewBox). Render it in a light
neutral: the official black mark disappears on the dark UI.

## Trademarks

Claude and Anthropic are trademarks of Anthropic PBC; OpenAI and Codex are trademarks of OpenAI;
Gemini and Google are trademarks of Google LLC. The CC0 license of simple-icons covers the SVG
files only, not the trademarks. The marks are used here only to identify which tool runs in a pane
(nominative use), unmodified in shape, never as AgentDeck's own branding, and never to suggest
endorsement. Before distributing AgentDeck publicly, check each brand's current guidelines
(simple-icons lists them per icon in `node_modules/simple-icons/data/simple-icons.json` under `guidelines`).
