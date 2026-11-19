---
name: agentdeck-frontend
description: Use to build or change a web UI component, form, layout, style, client state or data fetching in React, Vue, Svelte, Angular, Solid or HTML/CSS. Check accessibility and loading states.
---

# Frontend

## Before writing

- Find what exists: the component library or `components/` folder, design tokens (CSS variables,
  Tailwind config, theme file), the data-fetching pattern, routing, form handling. Use them the way
  neighbouring screens do. A second button, modal or color is a defect.
- Write the states the screen can be in, before the markup: loading, empty, error, success, partial,
  long text, many items, no permission, offline.

## Building

- **Semantic HTML first**: `button` for actions, `a` for navigation, `label` tied to every input,
  headings in order, lists as lists. A `div` with `onClick` is a bug.
- **Keyboard and screen readers**: everything reachable with Tab in a sensible order, visible focus,
  Escape closes overlays and returns focus, images have `alt` (empty for decoration), icon buttons
  have an accessible name, contrast at least 4.5:1 for text.
- **Forms**: native types (`email`, `number`, `tel`), `autocomplete`, validation on submit and on blur,
  the error next to its field and announced, the submit button disabled while sending, no double
  submit, input kept after an error.
- **State**: derive what can be derived; keep state in the lowest component that needs it; URL for
  what should survive a reload or be shared (filters, tabs, page); server data in the project's cache
  (TanStack Query, SWR, loaders) instead of copies in global state.
- **Data**: show cached data while refetching, cancel stale requests, handle every error the API
  contract lists with a message the user understands.
- **Layout**: mobile first, no fixed widths that overflow at 360px, `min-width: 0` on flex children
  that hold text, reserve space for images and async content (no layout shift), `100dvh` not `100vh`.
- **Performance**: no heavy work in render, stable keys on lists, virtualize lists over a few hundred
  rows, lazy-load routes and heavy widgets, images sized and lazy below the fold, no new dependency for
  something the platform does.
- **Text**: no hard-coded strings where the project uses i18n; dates and numbers through `Intl`.

## Check it in the panel

Open the page in the AgentDeck browser panel (the `browser` MCP tools), never another browser:

1. Each state on screen: force loading, empty and error (block the request or use bad input).
2. Keyboard only: Tab through, use the feature, Escape out.
3. Phone width with the panel's mobile preview, then desktop.
4. Console and network clean; a `lighthouse_audit` snapshot for accessibility.

Use the `agentdeck-web-check` and `agentdeck-mobile-check` skills for the full loop when installed.

Report: components reused or added, states handled, what you saw in the panel at each width.
