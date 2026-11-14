---
name: agentdeck-mobile-check
description: Check a web page at phone and tablet width in the AgentDeck browser panel - overflow, tap targets, text size, menus, forms, keyboard, safe areas, slow network. Use for responsive or mobile work, a mobile app's web preview (Expo web, Capacitor, PWA), or when asked if a page works on phones.
---

# Mobile check in the AgentDeck browser panel

Use only the `browser` MCP tools (the AgentDeck panel the user watches).

## Widths to check

| Device | Viewport | Why |
|---|---|---|
| Small phone | 360 x 740 | most Android, breaks first |
| Phone | 390 x 844 | iPhone 13-15 |
| Large phone | 430 x 932 | Pro Max, landscape-like spacing |
| Tablet | 768 x 1024 | layout breakpoint |

Use `emulate` with a mobile, touch viewport (sets touch events and the mobile user agent), or
`resize_page`. The user can also turn on the panel's mobile preview.

## Steps

1. Load the page (`navigate_page`), set the smallest width first.
2. `take_screenshot` of the first screen, then scroll (`evaluate_script` `window.scrollBy(0, 800)`)
   and screenshot again until the end of the page.
3. Measure instead of guessing, with `evaluate_script`:
   - horizontal overflow: `document.documentElement.scrollWidth > window.innerWidth`; find the
     culprit: `[...document.querySelectorAll('*')].filter(e => e.getBoundingClientRect().right > innerWidth).map(e => e.tagName + '.' + e.className)`;
   - tap targets under 44 x 44 px: same idea on `a, button, input, [role=button]`;
   - text under 16 px on inputs (iOS zooms the page on focus) and under 14 px for body text.
4. Use it: open the menu, dialogs, dropdowns; fill a form field; check the submit is still reachable
   and the fixed header/footer does not cover content.
5. Check the usual mobile breakers:
   - `<meta name="viewport" content="width=device-width, initial-scale=1">` present;
   - `100vh` on mobile: use `100dvh` / `100svh`;
   - notch: `env(safe-area-inset-*)` with `viewport-fit=cover`;
   - images with `max-width: 100%`, tables and code blocks scroll inside their own box;
   - long words and URLs: `overflow-wrap: anywhere`;
   - hover-only interactions have a tap equivalent;
   - flex/grid children with `min-width: 0` so text can shrink.
6. Slow network: `emulate` with a slow 4G/3G profile, reload, check loading states and that the page
   is usable before everything arrives.
7. Landscape: swap width and height once.
8. Fix, reload, check again at every width. Put the viewport back (`emulate` with no viewport, or
   `resize_page`) when done.

## Report

Widths checked, what broke (with the element), what you fixed, what you could not test (real
device, iOS Safari quirks, a real on-screen keyboard).
