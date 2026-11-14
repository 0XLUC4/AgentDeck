---
name: agentdeck-mobile-check
description: Check a web page at phone width in the AgentDeck browser panel - overflow, tap targets, text size, menus, forms. Use for responsive or mobile work, a mobile app's web preview, or when asked if a page works on phones.
---

# Mobile check in the AgentDeck browser panel

Use only the `browser` MCP tools (the AgentDeck panel the user watches).

1. Load the page (`navigate_page`). Switch to phone size: `emulate` with a phone viewport
   (390 x 844, mobile, touch) or `resize_page` to 390 x 844. The user can also turn on the panel's
   mobile preview.
2. `take_screenshot` of the first screen, then scroll (`evaluate_script` `window.scrollBy(0, 800)`)
   and screenshot again until the end of the page.
3. Check, with `evaluate_script` when the eye is not enough:
   - no horizontal scroll: `document.documentElement.scrollWidth <= window.innerWidth`;
   - tap targets at least 44 x 44 px, not overlapping;
   - body text at least 16 px, no text cut or overflowing its box;
   - menus, dialogs and forms usable: open them, fill a field, the keyboard does not hide the submit.
4. Fix, reload, check again. Put the viewport back (`emulate` with no viewport, or `resize_page`) when done.

Report the widths checked, what broke and what you fixed.
