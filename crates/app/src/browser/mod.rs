//! Browser panel for a Claude pane: a native webview (gpui-webview, Wry/WebView2) laid over the panel body,
//! driven by the agent through chrome-devtools-mcp on a per-pane DevTools port. URL bar, back/forward,
//! reload, close. The only web view in the app. The user can annotate elements of the page: the
//! notes go to the agent's prompt, to ask for precise changes.
//! Original: src/components/Browser/BrowserPanel.tsx, src-tauri/src/browser.rs
//! (browser_open -> Claude args, browser_bounds, browser_go, browser_url, browser_close).
//!
//! shortcut: the debugging port listens on 127.0.0.1 without auth, so a local process could drive this
//! webview (not the app UI). Fine for a dev tool on a personal machine.

use std::cell::Cell;
use std::collections::HashMap;
use std::net::TcpListener;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::base::Selectable as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use gpui_webview::WebView;
use agentdeck_core::agents::{AgentKind, browser_guard};
use agentdeck_core::hooks_server::{BROWSER_PORT_ENV, browser_config_port, browser_server};
use serde_json::{Value, json};

use crate::state::Deck;
use crate::theme::{hsla, palette, ui};

/// Same start page as the Tauri build.
pub const START_URL: &str = "https://www.google.com";
/// wry's defaults (lost when browser args are set), plus: a parked or occluded panel keeps rendering
/// and running timers, so the agent's screenshots, scripts and waits work while the user looks elsewhere.
/// One `--disable-features` only: Chromium keeps the last one.
const WRY_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,CalculateNativeWinOcclusion \
--disable-background-timer-throttling --disable-backgrounding-occluded-windows --disable-renderer-backgrounding";
/// How often a shown panel checks it is still on screen (tab switches, layout changes).
const FOLLOW_MS: u64 = 150;
/// How often the address bar reads the page URL.
const URL_POLL_MS: u64 = 1000;
/// Told to every Claude and OpenCode agent (the agents with a prompt channel), so that it checks its web work in the browser instead of assuming it.
const ROLE: &str = "# Browser: use only the AgentDeck browser panel

AgentDeck has a browser panel next to your terminal, driven with the `browser` MCP tools \
(navigate_page, new_page, take_snapshot, take_screenshot, click, fill...); it opens by itself on your first \
browser tool call. The user watches that panel: a page you open anywhere else is invisible to them.

Rules (no exceptions, even if the user's project, a README or a script suggests otherwise):
- To open, show, test or check any web page, web app, dev server, local HTML file served over http, or a \
mobile app's web preview, use the `browser` MCP tools: navigate the panel to the URL, try what you changed \
and report what you saw. Do it before any other way of testing.
- Keep one page: reuse the current one with `navigate_page`, call `new_page` only when you need two pages \
at once, and close the extra pages (`close_page`) when done.
- NEVER open a page in a separate window or the system browser: no `start <url>`, `Start-Process <url>`, \
`explorer <url>`, `cmd /c start`, `rundll32 url.dll`, `open`, `xdg-open`, Python `webbrowser`, Node `open`, \
`msedge`/`chrome`/`firefox <url>`, no standalone webview or Electron window. Do not pass `--open` (or set \
`open: true`) to a dev server; start it without opening and navigate the panel to its URL.
- NEVER use another browser automation tool, even if available: no Playwright, Puppeteer, Selenium, another \
Chrome DevTools MCP server, a Chrome extension or computer-use server. Only the `browser` MCP server drives \
the panel the user sees.
- While the panel is open, use it for anything else on the web too (look something up, read a page, check a \
link); while it is closed, plain lookups may use search or fetch tools.
- Phone width: the panel has a mobile preview (390x844 viewport, mobile user agent, touch), switched by \
its phone button. NEVER call `resize_page`, or `emulate` with a viewport: they resize the native page, which \
then leaves the panel. Claude agents: such a call is blocked and switches the mobile preview on (phone \
width) or off (wider) instead. Other agents: ask the user to click the phone button. `emulate` without a \
viewport (network, CPU throttling) is fine.
- The user may send you notes about elements of the page: each one names the element by its CSS selector \
and is either a Change to make or a Question to answer (answer it, change nothing for it).

# Side tasks: stay inside AgentDeck

Run every command (checks, tests, builds, the app, a dev server, a watcher) with your own shell tool, in \
this terminal: the user sees it there. Run a long-running one (dev server, watcher, the app) in the \
background with your shell tool and read its output from there. NEVER open an external console or terminal \
window: no `start cmd`, `start powershell`, `cmd /c start`, `Start-Process` without `-NoNewWindow`, `wt`, \
`conhost`, `CREATE_NEW_CONSOLE`, no new terminal tab or app. A window outside AgentDeck is one the user \
cannot see or stop from here.

These rules say where to test and run commands, not that you must: when your task says not to build, run \
or test (an AgentDeck orchestrator sends that to its worker agents), do not, the orchestrator verifies after.";

/// `BROWSER` of the agent's process: tools that honor it (Python `webbrowser`, Vite, create-react-app)
/// open their page in the panel (`POST <hook url>/open`, see `hooks_server::OPEN_PATH`) instead of the
/// system browser. Token and pane id come from the pane's env, so the file holds no secret.
// shortcut: Windows only, and a URL with `&` but no space is split by cmd before it gets here; port a
// small exe or a sh script if that matters.
const OPEN_CMD: &str = "@echo off\r\n\
rem AgentDeck: shows %1 in this pane's browser panel instead of the system browser.\r\n\
echo(%~1| curl.exe -s -m 3 -o NUL -H \"X-AgentDeck-Token: %AGENTDECK_HOOK_TOKEN%\" \
-H \"X-AgentDeck-Pane: %AGENTDECK_PANE_ID%\" --data-binary @- \"%AGENTDECK_HOOK_URL%/open\"\r\n";
/// Phone viewport of the mobile preview, in logical pixels.
const PHONE_WIDTH: f32 = 390.0;
const PHONE_HEIGHT: f32 = 844.0;
/// What the page is told in mobile preview, so that sites serve their phone layout.
const MOBILE_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 \
(KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1";

/// Run in the page: turns annotating on (nothing when it is already on). Hovering outlines an element; a
/// click outlines it and opens a popover under it to write a note, marked as a change to make or a
/// question. Added notes are kept in `window.__agentdeckNotes` with the element's selector, text and
/// HTML. The popover lives in a closed shadow root, out of reach of the page's styles. Notes are lost
/// when the page reloads. Icons are Lucide's (crosshair, message-square-plus, pencil, circle-question-mark).
const ANNOTATE_JS: &str = r##"(() => {
  if (window.__agentdeckAnnotate) return;
  const notes = window.__agentdeckNotes = window.__agentdeckNotes || [];
  const icon = (body) => `<svg xmlns='http://www.w3.org/2000/svg' width='14' height='14' viewBox='0 0 24 24' fill='none' stroke='currentColor' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'>${body}</svg>`;
  const addIcon = icon("<path d='M22 17a2 2 0 0 1-2 2H6.828a2 2 0 0 0-1.414.586l-2.202 2.202A.71.71 0 0 1 2 21.286V5a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2z'/><path d='M12 8v6'/><path d='M9 11h6'/>");
  const changeIcon = icon("<path d='M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z'/><path d='m15 5 4 4'/>");
  const questionIcon = icon("<circle cx='12' cy='12' r='10'/><path d='M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3'/><path d='M12 17h.01'/>");
  const host = document.createElement('agentdeck-annotate');
  host.style.cssText = 'all:initial;position:fixed;top:0;left:0;z-index:2147483647;';
  const root = host.attachShadow({ mode: 'closed' });
  root.innerHTML = `<style>
    * { box-sizing: border-box; font-family: system-ui, -apple-system, 'Segoe UI', sans-serif; }
    .box { position: fixed; display: none; pointer-events: none; border: 2px solid #3b82f6; border-radius: 6px; background: rgba(59,130,246,.1); }
    .pop { position: fixed; display: none; width: 340px; padding: 12px; border-radius: 10px; background: #2b2b2b; border: 1px solid rgba(255,255,255,.1); box-shadow: 0 12px 32px rgba(0,0,0,.45); color: #ececec; font-size: 13px; }
    .head { display: flex; align-items: center; gap: 8px; min-width: 0; margin-bottom: 10px; }
    .tag { flex: none; padding: 1px 6px; border-radius: 4px; background: rgba(255,255,255,.08); color: #bdbdbd; font: 11px ui-monospace, Consolas, monospace; }
    .title { font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    textarea { display: block; width: 100%; height: 100px; resize: none; padding: 8px 10px; border-radius: 8px; border: 1px solid #5c5c5c; background: #161616; color: #ececec; font-size: 13px; line-height: 1.4; outline: none; }
    textarea:focus { border-color: #8c8c8c; }
    button { display: inline-flex; align-items: center; justify-content: center; gap: 6px; padding: 6px 10px; border: 0; border-radius: 6px; background: none; color: #a8a8a8; font-size: 13px; cursor: pointer; }
    button:hover { color: #fff; }
    .seg { display: flex; margin-top: 10px; padding: 2px; border-radius: 8px; background: #1f1f1f; border: 1px solid rgba(255,255,255,.08); }
    .seg button { flex: 1; }
    .seg button.on { background: #404040; color: #fff; }
    .foot { display: flex; justify-content: flex-end; gap: 8px; margin-top: 12px; }
    .add { background: #ececec; color: #111; font-weight: 500; }
    .add:hover { color: #111; background: #fff; }
    .add:disabled { opacity: .5; cursor: default; }
    .kbd { padding: 1px 5px; border-radius: 4px; background: rgba(0,0,0,.08); color: #555; font-size: 11px; }
  </style>
  <div class='box'></div>
  <div class='pop'>
    <div class='head'><span class='tag'></span><span class='title'></span></div>
    <textarea placeholder='Add feedback for the agent…'></textarea>
    <div class='seg'><button data-kind='change'>${changeIcon}Change</button><button data-kind='question'>${questionIcon}Question</button></div>
    <div class='foot'><button class='cancel'>Cancel</button><button class='add'>${addIcon}Add<span class='kbd'>Ctrl ↵</span></button></div>
  </div>`;
  const $ = (s) => root.querySelector(s);
  const box = $('.box'), pop = $('.pop'), text = $('textarea'), add = $('.add'), kinds = root.querySelectorAll('.seg button');
  let hovered = null, target = null, kind = 'change';
  const clip = (s, n) => (s || '').replace(/\s+/g, ' ').trim().slice(0, n);
  const selector = (el) => {
    const parts = [];
    while (el && el.nodeType === 1 && parts.length < 5) {
      let s = el.tagName.toLowerCase();
      if (el.id) { parts.unshift(s + '#' + el.id); break; }
      const cls = [...el.classList].slice(0, 2).join('.');
      if (cls) s += '.' + cls;
      const same = el.parentElement ? [...el.parentElement.children].filter(c => c.tagName === el.tagName) : [];
      if (same.length > 1) s += ':nth-of-type(' + (same.indexOf(el) + 1) + ')';
      parts.unshift(s);
      el = el.parentElement;
    }
    return parts.join(' > ');
  };
  // What the user sees of the element: its label, then its value or text, e.g. ZIP / Postal "94105".
  const describe = (el) => {
    const field = /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName);
    const attr = (a) => el.getAttribute(a);
    const label = clip((el.labels && el.labels[0] && el.labels[0].innerText) || attr('aria-label') || attr('placeholder') || attr('name') || attr('alt') || attr('title'), 40);
    const value = clip(field ? el.value : el.innerText, 40);
    return [label, value && '"' + value + '"'].filter(Boolean).join(' ');
  };
  const outline = (el) => {
    if (!el || !el.isConnected) { box.style.display = 'none'; return; }
    const r = el.getBoundingClientRect();
    Object.assign(box.style, { display: 'block', left: r.left - 3 + 'px', top: r.top - 3 + 'px', width: r.width + 6 + 'px', height: r.height + 6 + 'px' });
  };
  // Under the element, or above it when there is no room below; always inside the viewport.
  const place = () => {
    outline(target || hovered);
    if (!target) return;
    const r = target.getBoundingClientRect(), h = pop.offsetHeight, gap = 8;
    let top = r.bottom + gap;
    if (top + h > innerHeight - gap && r.top - gap - h >= gap) top = r.top - gap - h;
    top = Math.max(gap, Math.min(top, innerHeight - h - gap));
    const left = Math.max(gap, Math.min(r.left, innerWidth - pop.offsetWidth - gap));
    Object.assign(pop.style, { top: top + 'px', left: left + 'px' });
  };
  const setKind = (k) => { kind = k; kinds.forEach(b => b.classList.toggle('on', b.dataset.kind === k)); };
  const open = (el) => {
    target = el;
    setKind('change');
    text.value = '';
    add.disabled = true;
    $('.tag').textContent = el.tagName.toLowerCase();
    $('.title').textContent = describe(el);
    pop.style.display = 'block';
    place();
    text.focus();
  };
  const close = () => { target = null; pop.style.display = 'none'; place(); };
  const submit = () => {
    const note = text.value.trim();
    if (!note || !target) return;
    notes.push({ selector: selector(target), text: (target.innerText || target.value || '').trim().slice(0, 120), html: target.outerHTML.slice(0, 400), note: note.slice(0, 1000), kind, url: location.href });
    close();
  };
  kinds.forEach(b => b.addEventListener('click', () => { setKind(b.dataset.kind); text.focus(); }));
  $('.cancel').addEventListener('click', close);
  add.addEventListener('click', submit);
  text.addEventListener('input', () => { add.disabled = !text.value.trim(); });
  text.addEventListener('keydown', (e) => {
    e.stopPropagation();
    if (e.key === 'Escape') { e.preventDefault(); close(); }
    else if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); submit(); }
  });
  // The page gets no press or click while annotating; the popover gets its own.
  const ours = (e) => e.composedPath().includes(host);
  const block = (e) => { if (!ours(e)) { e.preventDefault(); e.stopPropagation(); } };
  const click = (e) => { if (!ours(e)) { block(e); open(e.target); } };
  const move = (e) => { if (!ours(e)) { hovered = e.target; if (!target) outline(hovered); } };
  const events = [['mousemove', move], ['pointerdown', block], ['mousedown', block], ['click', click]];
  events.forEach(([name, f]) => document.addEventListener(name, f, true));
  addEventListener('scroll', place, true);
  addEventListener('resize', place);
  document.documentElement.append(host);
  window.__agentdeckAnnotate = { stop() {
    events.forEach(([name, f]) => document.removeEventListener(name, f, true));
    removeEventListener('scroll', place, true);
    removeEventListener('resize', place);
    host.remove();
    delete window.__agentdeckAnnotate;
  } };
})()"##;

/// Run in the page: turns annotating off, keeping the notes.
const STOP_ANNOTATE_JS: &str = "window.__agentdeckAnnotate && window.__agentdeckAnnotate.stop()";

/// Reads the notes and empties the list.
const TAKE_NOTES_JS: &str = "(() => { const n = window.__agentdeckNotes || []; window.__agentdeckNotes = []; return JSON.stringify(n); })()";

/// The prompt carrying the page notes, or None when there is none. `json` is what the page
/// answered: the notes as JSON, possibly wrapped in a JSON string once more by the webview.
// The page owns that list, so a hostile site could write notes of its own: the prompt is typed
// into the agent's box without being submitted, for the user to read first.
fn notes_prompt(json: &str) -> Option<String> {
    let mut value: Value = serde_json::from_str(json).ok()?;
    if let Some(inner) = value.as_str() {
        value = serde_json::from_str(inner).ok()?;
    }
    // One line each, without control characters: a note must not break out of its place.
    let clean = |v: &Value, max: usize| -> String {
        let text = v.as_str().unwrap_or("");
        text.chars().map(|c| if c.is_control() { ' ' } else { c }).take(max).collect::<String>().trim().to_owned()
    };
    let notes: Vec<String> = value
        .as_array()?
        .iter()
        .take(20)
        .enumerate()
        .filter_map(|(i, n)| {
            let note = clean(&n["note"], 1000);
            // A question asks for an answer only; anything else is a change to make.
            let kind = if n["kind"] == "question" { "Question" } else { "Change" };
            (!note.is_empty()).then(|| {
                format!(
                    "{}. {}\n   Element: `{}` (\"{}\")\n   HTML: `{}`\n   {kind}: {note}",
                    i + 1,
                    clean(&n["url"], 300),
                    clean(&n["selector"], 300),
                    clean(&n["text"], 120),
                    clean(&n["html"], 400),
                )
            })
        })
        .collect();
    (!notes.is_empty())
        .then(|| {
            format!(
                "Notes made on the page in the browser. Make each Change; answer each Question without changing anything:\n\n{}\n",
                notes.join("\n\n")
            )
        })
}

thread_local! {
    /// Frames drawn by the main window (see `frame_drawn`).
    static FRAME: Cell<u64> = const { Cell::new(0) };
    /// Last frame a GPUI popup was rendered in (see `popup_shown`).
    static POPUP_FRAME: Cell<u64> = const { Cell::new(u64::MAX) };
}

/// Called while rendering an open popup menu of the main window: webviews hide this frame, as for a dialog.
pub fn popup_shown() {
    POPUP_FRAME.with(|p| p.set(FRAME.with(Cell::get)));
}

/// Called once per drawn frame of the main window (its root view's render). A panel that was not
/// drawn in the last frame has left the screen: its native webview is hidden, without forcing redraws.
pub fn frame_drawn() {
    FRAME.with(|f| f.set(f.get() + 1));
}

pub fn init(cx: &mut App) {
    cx.set_global(Browsers::default());
}

/// One pane's webview. The WebContext is kept alive with it (it owns the profile directory).
struct Browser {
    view: Entity<WebView>,
    port: u16,
    /// Mobile preview: the page is laid out as a phone viewport, with mobile emulation.
    mobile: bool,
    /// Set when a WebView2 process failed (GPU, renderer): the panel revives the view (see `revive`).
    crashed: Rc<Cell<bool>>,
    _context: wry::WebContext,
}

/// pane id -> its browser webview (children of the main window).
#[derive(Default)]
struct Browsers(HashMap<String, Browser>);

impl Global for Browsers {}

fn browser_view(pane_id: &str, cx: &App) -> Result<Entity<WebView>, String> {
    cx.try_global::<Browsers>()
        .and_then(|b| b.0.get(pane_id))
        .map(|b| b.view.clone())
        .ok_or_else(|| "no browser for this agent".into())
}

fn valid_id(pane_id: &str) -> Result<(), String> {
    let ok = (1..=64).contains(&pane_id.len())
        && pane_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    ok.then_some(()).ok_or_else(|| "invalid pane id".into())
}

/// Creates the pane's browser webview in `window` (nothing to do when it exists), hidden until its panel
/// is drawn, on the port its running agent was given by `args`.
pub fn open(pane_id: &str, data_dir: &Path, window: &mut Window, cx: &mut App) -> Result<(), String> {
    valid_id(pane_id)?;
    if cx.try_global::<Browsers>().is_some_and(|b| b.0.contains_key(pane_id)) {
        return Ok(());
    }
    let dir = data_dir.join("browser");
    let told = std::fs::read_to_string(dir.join(format!("{pane_id}.json"))).ok().and_then(|text| browser_config_port(&text));
    let port = match told {
        Some(port) => port,
        None => free_port()?,
    };
    // Different browser args need their own WebView2 environment, hence data directory.
    let mut context = wry::WebContext::new(Some(dir.join("profiles").join(pane_id)));
    // Theme background, not WebView2's default white, until the page paints (no white flash).
    let [_, r, g, b] = palette(cx).base.to_be_bytes();
    let builder = wry::WebViewBuilder::new_with_web_context(&mut context)
        .with_url(START_URL)
        // Opened by an agent's tool call while the user types elsewhere: never take the focus.
        .with_focused(false)
        .with_background_color((r, g, b, 255));
    #[cfg(windows)]
    let builder = {
        use wry::WebViewBuilderExtWindows as _;
        builder.with_additional_browser_args(format!("{WRY_ARGS} --remote-debugging-port={port}"))
    };
    let webview = WebView::build(builder, window, cx).map_err(|e| format!("cannot open the browser: {e}"))?;
    // Off-screen until the panel's prepaint shows it at its bounds; usable by the agent meanwhile.
    park(&webview);
    let crashed = Rc::new(Cell::new(false));
    #[cfg(windows)]
    watch_crashes(webview.raw(), crashed.clone());
    let view = cx.new(|_| webview);
    let browser = Browser { view, port, mobile: false, crashed, _context: context };
    cx.default_global::<Browsers>().0.insert(pane_id.to_owned(), browser);
    Ok(())
}

/// Flags the view when a WebView2 process fails: a GPU process restart leaves the view black until it
/// is hidden and shown again. A dead or hung renderer also gets its page reloaded.
#[cfg(windows)]
fn watch_crashes(webview: &wry::WebView, crashed: Rc<Cell<bool>>) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
    };
    use wry::WebViewExtWindows as _;
    let handler = webview2_com::ProcessFailedEventHandler::create(Box::new(move |page, args| {
        let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
        if let Some(args) = args {
            // SAFETY: COM calls on the UI thread that owns the webview, as WebView2 events are.
            unsafe { args.ProcessFailedKind(&mut kind)? };
        }
        eprintln!("browser: WebView2 process failed (kind {}), reviving the view", kind.0);
        if matches!(kind, COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED | COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE)
            && let Some(page) = page
        {
            // SAFETY: as above.
            unsafe { page.Reload()? };
        }
        crashed.set(true);
        Ok(())
    }));
    let mut token = 0i64;
    // SAFETY: registers the handler on the UI thread; it lives as long as the webview.
    if let Err(e) = unsafe { webview.webview().add_ProcessFailed(&handler, &mut token) } {
        eprintln!("browser: cannot watch WebView2 failures: {e}");
    }
}

/// Hides and shows the native view again, which makes WebView2 rebuild its visual (a page left black by
/// a GPU process restart, sleep, a display change). The next prepaint puts it back at its bounds.
fn revive(view: &WebView) {
    let _ = view.set_visible(false);
    let _ = view.set_visible(true);
    #[cfg(windows)]
    {
        use wry::WebViewExtWindows as _;
        // SAFETY: a COM call on the UI thread that owns the webview.
        let _ = unsafe { view.raw().controller().NotifyParentWindowPositionChanged() };
    }
}

/// Agents that can be given the browser MCP server at spawn. Grok only reads MCP servers from its
/// global or project config file, which AgentDeck does not edit.
pub fn supported(agent: AgentKind) -> bool {
    matches!(agent, AgentKind::Claude | AgentKind::Codex | AgentKind::Gemini | AgentKind::OpenCode)
}

/// Writes the pane's MCP config and returns the args and env that connect `agent` to its browser, each
/// through its own channel: Claude `--mcp-config` (+ the role prompt), Codex `-c mcp_servers.browser`,
/// Gemini its AgentDeck defaults layer (`agents::gemini_env`, port from the env), OpenCode
/// `OPENCODE_CONFIG_CONTENT` (+ the role as instructions). Every such pane starts with them:
/// chrome-devtools-mcp only connects on a tool call, so the webview is opened and closed later
/// without restarting the agent.
// shortcut: the port is only bound when the panel is opened, another program could take it meanwhile;
// reserve it (or restart on a clash) if that ever shows up.
pub fn args(pane_id: &str, data_dir: &Path, agent: AgentKind, cwd: &Path, cx: &App) -> Result<(Vec<String>, Vec<(String, String)>), String> {
    valid_id(pane_id)?;
    let dir = data_dir.join("browser");
    let port = match cx.try_global::<Browsers>().and_then(|b| b.0.get(pane_id)) {
        Some(browser) => browser.port,
        None => free_port()?,
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let config = dir.join(format!("{pane_id}.json"));
    let text = serde_json::to_string_pretty(&config_for(port)).map_err(|e| e.to_string())?;
    std::fs::write(&config, text).map_err(|e| format!("{}: {e}", config.display()))?;
    // A file, not an inline argument: spawn args cannot hold quotes on Windows.
    let role = dir.join("role.md");
    std::fs::write(&role, ROLE).map_err(|e| format!("{}: {e}", role.display()))?;
    let (args, mut env) = wiring(agent, port, &config, &role, cwd);
    if cfg!(windows) {
        let opener = dir.join("open.cmd");
        std::fs::write(&opener, OPEN_CMD).map_err(|e| format!("{}: {e}", opener.display()))?;
        env.push(("BROWSER".into(), opener.to_string_lossy().into_owned()));
    }
    Ok((args, env))
}

/// Args and env that give `agent` the browser server on `port` (see `args`).
fn wiring(agent: AgentKind, port: u16, config: &Path, role: &Path, cwd: &Path) -> (Vec<String>, Vec<(String, String)>) {
    let path = |p: &Path| p.to_string_lossy().into_owned();
    let server = browser_server(&format!("http://127.0.0.1:{port}"));
    match agent {
        AgentKind::Claude => {
            (vec!["--mcp-config".into(), path(config), "--append-system-prompt-file".into(), path(role)], Vec::new())
        }
        // TOML literal strings and no spaces survive cmd.exe + the npm .cmd shim (see `hub::agent_args`).
        AgentKind::Codex => {
            let quoted = |v: &Value| format!("'{}'", v.as_str().unwrap_or_default());
            let args: Vec<String> = server["args"].as_array().into_iter().flatten().map(quoted).collect();
            let entry = format!("mcp_servers.browser={{command={},args=[{}]}}", quoted(&server["command"]), args.join(","));
            // No blocking hook in Codex: its other browser servers are off for this agent.
            let mut all = vec!["-c".into(), entry];
            all.extend(browser_guard::codex_args(cwd));
            (all, Vec::new())
        }
        AgentKind::Gemini => (Vec::new(), vec![(BROWSER_PORT_ENV.into(), port.to_string())]),
        AgentKind::OpenCode => {
            let command: Vec<Value> =
                std::iter::once(server["command"].clone()).chain(server["args"].as_array().into_iter().flatten().cloned()).collect();
            // No blocking hook in OpenCode: its other browser servers are off for this agent.
            let mut mcp = browser_guard::opencode_off();
            mcp.insert("browser".into(), json!({ "type": "local", "command": command }));
            let content = json!({
                "mcp": mcp,
                "instructions": [path(role)],
            });
            (Vec::new(), vec![("OPENCODE_CONFIG_CONTENT".into(), content.to_string())])
        }
        _ => (Vec::new(), Vec::new()),
    }
}

/// Hides the webview by moving it off-screen, still natively visible: WebView2 freezes a hidden view
/// (no paint, throttled timers, screenshots that never come), and the agent drives it while its panel
/// is not on screen. The wrapper's own flag goes off, so the panel's next prepaint puts it back.
fn park(view: &WebView) {
    let _ = view.set_visible(false);
    let raw = view.raw();
    // Keeps the last shown size: pages lay out and screenshots come at the panel's size.
    let size = raw
        .bounds()
        .map(|b| b.size)
        .ok()
        .filter(|size| match size {
            wry::dpi::Size::Physical(p) => p.width > 0 && p.height > 0,
            wry::dpi::Size::Logical(l) => l.width > 0.0 && l.height > 0.0,
        })
        .unwrap_or_else(|| wry::dpi::LogicalSize::new(1280.0, 800.0).into());
    let position = wry::dpi::LogicalPosition::new(-30000.0, -30000.0).into();
    let _ = raw.set_bounds(wry::Rect { position, size });
    let _ = raw.set_visible(true);
}

/// Header controls: "back", "forward", "reload", or an http(s) URL to open.
pub fn go(pane_id: &str, to: &str, cx: &mut App) -> Result<(), String> {
    let view = browser_view(pane_id, cx)?;
    let raw = view.read(cx).raw();
    let result = match to {
        "back" => raw.evaluate_script("history.back()"),
        "forward" => raw.evaluate_script("history.forward()"),
        "reload" => raw.reload(),
        _ => raw.load_url(parse_url(to)?.as_str()),
    };
    // A black view does not come back with the page: reload also rebuilds the view.
    if to == "reload" && view.read(cx).visible() {
        revive(view.read(cx));
        cx.refresh_windows();
    }
    result.map_err(|e| e.to_string())
}

/// Current URL of the pane's browser, for the header.
pub fn url(pane_id: &str, cx: &App) -> Result<String, String> {
    browser_view(pane_id, cx)?.read(cx).raw().url().map_err(|e| e.to_string())
}

/// Closes the pane's browser for good (pane closed or browser turned off).
pub fn close(pane_id: &str, cx: &mut App) {
    // Dropping the entity hides the native view; it is destroyed with its last handle.
    if cx.has_global::<Browsers>() {
        cx.global_mut::<Browsers>().0.remove(pane_id);
    }
}

/// "example.com" or "https://example.com/x"; only http(s).
fn parse_url(text: &str) -> Result<url::Url, String> {
    let text = text.trim();
    let full = if text.contains("://") {
        text.to_owned()
    } else {
        format!("https://{text}")
    };
    let parsed = url::Url::parse(&full).map_err(|_| format!("not a URL: {text}"))?;
    if matches!(parsed.scheme(), "http" | "https") {
        Ok(parsed)
    } else {
        Err("only http and https URLs".into())
    }
}

fn free_port() -> Result<u16, String> {
    // shortcut: the port is released before WebView2 binds it; a race is unlikely on 127.0.0.1.
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| format!("no free port: {e}"))
}

fn config_for(port: u16) -> Value {
    json!({ "mcpServers": { "browser": browser_server(&format!("http://127.0.0.1:{port}")) } })
}

/// DevTools protocol calls that turn mobile emulation on or off: user agent and touch.
fn mobile_emulation(on: bool) -> [(&'static str, Value); 2] {
    [
        // An empty user agent gives the page its own back.
        ("Emulation.setUserAgentOverride", json!({ "userAgent": if on { MOBILE_UA } else { "" } })),
        ("Emulation.setTouchEmulationEnabled", json!({ "enabled": on })),
    ]
}

/// Switches the page to mobile emulation or back, and reloads it so the site answers for that device.
// shortcut: emulation on Windows (WebView2) only; elsewhere the preview is the narrow viewport alone.
fn set_mobile(webview: &wry::WebView, on: bool) {
    #[cfg(windows)]
    {
        use wry::WebViewExtWindows as _;
        let page = webview.webview();
        for (method, params) in mobile_emulation(on) {
            let method = windows_core::HSTRING::from(method);
            let params = windows_core::HSTRING::from(params.to_string());
            // SAFETY: a COM call on the UI thread that owns the webview.
            let _ = unsafe { page.CallDevToolsProtocolMethod(&method, &params, None) };
        }
    }
    let _ = webview.reload();
}

fn is_mobile(pane_id: &str, cx: &App) -> bool {
    cx.try_global::<Browsers>().and_then(|b| b.0.get(pane_id)).is_some_and(|b| b.mobile)
}

/// Turns the pane's mobile preview on or off: from the panel's phone button, or when the agent tried to
/// resize the page itself (`hooks_server::viewport_request`).
pub fn set_mobile_preview(pane_id: &str, on: bool, cx: &mut App) -> Result<(), String> {
    let browser = cx.default_global::<Browsers>().0.get_mut(pane_id).ok_or("no browser for this agent")?;
    if browser.mobile == on {
        return Ok(());
    }
    browser.mobile = on;
    let view = browser.view.clone();
    set_mobile(view.read(cx).raw(), on);
    cx.refresh_windows();
    Ok(())
}

/// Emitted when the user closed the browser for good: the pane is saved with `browser: false` and its
/// webview is gone. The agent keeps running.
#[derive(Debug, Clone)]
pub struct BrowserRemoved {
    pub pane_id: String,
}

/// Dock panel of a pane's browser: address bar header, and a body the native webview is laid over.
pub struct BrowserPanel {
    pub deck: Entity<Deck>,
    pub pane_id: String,
    address: Entity<InputState>,
    editing: bool,
    /// The page is in annotating mode (see `ANNOTATE_JS`).
    annotating: bool,
    /// Set by the body's prepaint: the last frame (see `FRAME`) the panel was drawn in.
    seen_frame: Rc<Cell<u64>>,
    /// The webview was hidden natively because the panel left the screen.
    hidden: Rc<Cell<bool>>,
    _subscriptions: Vec<Subscription>,
    _tasks: Vec<Task<()>>,
}

impl EventEmitter<BrowserRemoved> for BrowserPanel {}

impl BrowserPanel {
    pub fn new(deck: Entity<Deck>, pane_id: impl Into<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let pane_id = pane_id.into();
        let address = cx.new(|cx| InputState::new(window, cx));
        let subscriptions = vec![
            cx.subscribe_in(&address, window, |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Focus => this.editing = true,
                InputEvent::Blur => this.editing = false,
                InputEvent::PressEnter { .. } => {
                    this.editing = false;
                    let to = input.read(cx).value().to_string();
                    this.go(&to, cx);
                    window.blur(cx);
                }
                InputEvent::Change => {}
            }),
            cx.observe(&deck, |_, _, cx| cx.notify()),
            // Leaving the tree (panel closed) must not leave the native view over the window.
            cx.on_release(|this, cx| this.hide_native(cx)),
        ];
        let follow = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(FOLLOW_MS)).await;
                if this.update(cx, |this, cx| this.follow(cx)).is_err() {
                    break;
                }
            }
        });
        let read_url = cx.spawn_in(window, async move |this, cx| {
            loop {
                if this.update_in(cx, |this, window, cx| this.read_url(window, cx)).is_err() {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(URL_POLL_MS)).await;
            }
        });
        Self {
            deck,
            pane_id,
            address,
            editing: false,
            annotating: false,
            seen_frame: Rc::new(Cell::new(0)),
            hidden: Rc::new(Cell::new(true)),
            _subscriptions: subscriptions,
            _tasks: vec![follow, read_url],
        }
    }

    fn go(&self, to: &str, cx: &mut Context<Self>) {
        if let Err(e) = go(&self.pane_id, to, cx) {
            self.deck.update(cx, |d, cx| d.toast(format!("Browser: {e}"), cx));
        }
    }

    /// Turns annotating on or off in the page.
    fn set_annotating(&mut self, on: bool, cx: &mut Context<Self>) {
        let script = if on { ANNOTATE_JS } else { STOP_ANNOTATE_JS };
        let run = browser_view(&self.pane_id, cx)
            .and_then(|view| view.read(cx).raw().evaluate_script(script).map_err(|e| e.to_string()));
        if let Err(e) = &run {
            self.deck.update(cx, |d, cx| d.toast(format!("Browser: {e}"), cx));
        }
        self.annotating = on && run.is_ok();
        cx.notify();
    }

    /// Turns the mobile preview on or off.
    fn toggle_mobile(&mut self, cx: &mut Context<Self>) {
        let on = !is_mobile(&self.pane_id, cx);
        if let Err(e) = set_mobile_preview(&self.pane_id, on, cx) {
            self.deck.update(cx, |d, cx| d.toast(format!("Browser: {e}"), cx));
        }
        cx.notify();
    }

    /// Types the page notes into the agent's prompt, for the user to read and submit.
    fn send_notes(&mut self, cx: &mut Context<Self>) {
        let (tx, rx) = smol::channel::bounded::<String>(1);
        let asked = browser_view(&self.pane_id, cx).and_then(|view| {
            view.read(cx)
                .raw()
                .evaluate_script_with_callback(TAKE_NOTES_JS, move |json| {
                    let _ = tx.try_send(json);
                })
                .map_err(|e| e.to_string())
        });
        if let Err(e) = asked {
            self.deck.update(cx, |d, cx| d.toast(format!("Browser: {e}"), cx));
            return;
        }
        let pane_id = self.pane_id.clone();
        cx.spawn(async move |this, cx| {
            let Ok(json) = rx.recv().await else { return };
            let _ = this.update(cx, |this, cx| match notes_prompt(&json) {
                Some(prompt) => crate::orchestrator::runtime::send_prompt(&pane_id, &prompt, false, cx),
                None => this.deck.update(cx, |d, cx| {
                    d.toast("No note on this page: turn annotating on and click an element.", cx)
                }),
            });
        })
        .detach();
    }

    fn read_url(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing || self.hidden.get() {
            return;
        }
        let Ok(current) = url(&self.pane_id, cx) else { return };
        if self.address.read(cx).value() != current {
            self.address.update(cx, |input, cx| input.set_value(current, window, cx));
        }
    }

    fn hide_native(&self, cx: &App) {
        if let Ok(view) = browser_view(&self.pane_id, cx) {
            park(view.read(cx));
        }
        self.hidden.set(true);
    }

    /// While shown, hides the webview when the last drawn frame left the body out (tab switch, layout
    /// change). Views are re-rendered every frame, so no redraw is forced: that lagged and flickered.
    fn follow(&mut self, cx: &mut Context<Self>) {
        // A frame that draws the body again shows it again.
        if !self.hidden.get() && self.seen_frame.get() != FRAME.with(Cell::get) {
            self.hide_native(cx);
        }
        // A failed WebView2 process left the view black; a hidden view is revived when shown again.
        let crashed = cx.try_global::<Browsers>().and_then(|b| b.0.get(&self.pane_id)).map(|b| (b.view.clone(), b.crashed.clone()));
        if let Some((view, crashed)) = crashed
            && crashed.replace(false)
            && !self.hidden.get()
        {
            revive(view.read(cx));
            cx.notify();
        }
    }

    fn remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self
            .deck
            .read(cx)
            .state
            .panes
            .get(&self.pane_id)
            .map(|p| p.spec.title.clone())
            .unwrap_or_else(|| "Agent".into());
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Close {title}'s browser?"),
            None,
            &["Close browser", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                let pane_id = this.pane_id.clone();
                this.deck.update(cx, |d, cx| {
                    d.state.update_pane(&pane_id, |p| p.spec.browser = false);
                    d.save(cx);
                    cx.notify();
                });
                close(&pane_id, cx);
                cx.emit(BrowserRemoved { pane_id });
            });
        })
        .detach();
    }

    fn header(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let title = self.deck.read(cx).state.panes.get(&self.pane_id).map(|p| p.spec.title.clone());
        let nav = |id: &'static str, icon: IconName, tip: &'static str, to: &'static str| {
            Button::new(id)
                .ghost()
                
                .icon(icon)
                .tooltip(tip)
                .on_click(cx.listener(move |this, _, _, cx| this.go(to, cx)))
        };
        let back = nav("browser-back", IconName::ArrowLeft, "Back", "back");
        let forward = nav("browser-forward", IconName::ArrowRight, "Forward", "forward");
        let reload = nav("browser-reload", IconName::RotateCw, "Reload", "reload");
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(ui(6.0))
            .h(ui(46.0))
            .px(ui(12.0))
            .min_w_0()
            .bg(hsla(p.panel))
            .border_b_1()
            .border_color(hsla(p.line))
            .text_size(ui(14.0))
            .text_color(hsla(p.fg))
            .child(back)
            .child(forward)
            .child(reload)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(ui(14.0))
                    .child(Input::new(&self.address)),
            )
            .child(
                Button::new("browser-mobile")
                    .ghost()
                    .icon(IconName::Smartphone)
                    .selected(is_mobile(&self.pane_id, cx))
                    .tooltip("Mobile preview: show the page at phone width")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_mobile(cx))),
            )
            .child(
                Button::new("browser-annotate")
                    .ghost()
                    .icon(IconName::MessageSquarePlus)
                    .selected(self.annotating)
                    .tooltip("Annotate: click an element of the page and add feedback for the agent")
                    .on_click(cx.listener(|this, _, _, cx| this.set_annotating(!this.annotating, cx))),
            )
            .child(
                Button::new("browser-send-notes")
                    .ghost()
                    
                    .icon(IconName::Send)
                    .tooltip("Put the page notes in the agent's prompt")
                    .on_click(cx.listener(|this, _, _, cx| this.send_notes(cx))),
            )
            .when(title.is_some(), |el| {
                el.child(
                    crate::panes::close_button("browser-close", "Close browser").on_click({
                        let this = cx.entity().downgrade();
                        move |window, cx| {
                            let _ = this.update(cx, |this, cx| this.remove(window, cx));
                        }
                    }),
                )
            })
    }

    /// Shown under the header while annotating: what to do, and a way out.
    fn annotate_bar(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(ui(8.0))
            .h(ui(34.0))
            .px(ui(12.0))
            .bg(hsla(p.panel))
            .border_b_1()
            .border_color(hsla(p.line))
            .text_size(ui(13.0))
            .text_color(hsla(p.subtle))
            .child(IconName::Crosshair)
            .child(div().flex_1().min_w_0().truncate().child("Click an element to add feedback for the agent."))
            .child(
                Button::new("browser-annotate-cancel")
                    .ghost()
                    .label("Cancel")
                    .on_click(cx.listener(|this, _, _, cx| this.set_annotating(false, cx))),
            )
    }
}

impl Render for BrowserPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let body = match browser_view(&self.pane_id, cx) {
            Ok(view) => {
                let (seen_frame, hidden) = (self.seen_frame.clone(), self.hidden.clone());
                let webview = view.clone();
                // Runs before the webview's own prepaint, which positions and shows the native view.
                let guard = canvas(
                    move |_, window, cx| {
                        seen_frame.set(FRAME.with(Cell::get));
                        hidden.set(false);
                        // A native webview sits above every GPUI layer: hide it while a dialog is open.
                        // ...or while a popup (menu) was drawn this frame (`popup_shown`).
                        let dialog = window.has_active_dialog(cx)
                            || POPUP_FRAME.with(Cell::get) == FRAME.with(Cell::get);
                        webview.update(cx, |w, _| match (w.visible(), dialog) {
                            (true, true) => park(w),
                            (false, false) => {
                                let _ = w.set_visible(true);
                            }
                            _ => {}
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full();
                // Mobile preview: a phone viewport centered in the body, the rest keeps the panel background.
                let page = match is_mobile(&self.pane_id, cx) {
                    true => div().w(px(PHONE_WIDTH)).max_w_full().h_full().max_h(px(PHONE_HEIGHT)),
                    false => div().size_full(),
                };
                div().relative().flex().justify_center().flex_1().min_h_0().child(guard).child(page.child(view))
            }
            Err(_) => div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_size(ui(12.0))
                .text_color(hsla(p.subtle))
                .child("Starting the browser…"),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(hsla(p.base))
            .child(self.header(cx))
            .when(self.annotating, |el| el.child(self.annotate_bar(cx)))
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentKind, MOBILE_UA, Path, Value, browser_config_port, config_for, mobile_emulation, notes_prompt, parse_url, valid_id, wiring};

    #[test]
    fn page_notes_become_a_prompt() {
        let notes = r##"[{"selector": "form > input#zip", "text": "94105", "html": "<input id=\"zip\">", "note": "Accept Canadian\ncodes", "url": "http://localhost:3000/checkout"}, {"note": "  "}, {"note": "Why is this required?", "kind": "question"}]"##;
        let prompt = notes_prompt(notes).unwrap();
        assert!(prompt.contains("1. http://localhost:3000/checkout"));
        assert!(prompt.contains("Element: `form > input#zip` (\"94105\")"));
        // A line break in a note cannot start a line of its own; no kind is a change.
        assert!(prompt.contains("Change: Accept Canadian codes"));
        assert!(prompt.contains("Question: Why is this required?"));
        // The webview hands the JSON back as a JSON string.
        let wrapped = serde_json::to_string(notes).unwrap();
        assert_eq!(notes_prompt(&wrapped), Some(prompt));
        assert_eq!(notes_prompt("[]"), None);
        assert_eq!(notes_prompt("not json"), None);
    }

    #[test]
    fn config_connects_to_the_webview_port() {
        let c = config_for(9333);
        let args: Vec<&str> = c["mcpServers"]["browser"]["args"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        // Plain node on the cached script, or npx when it is not cached yet.
        assert!(args.ends_with(&["--no-usage-statistics", "--browserUrl", "http://127.0.0.1:9333"]));
        assert!(args[0].ends_with("chrome-devtools-mcp.js") || args.contains(&agentdeck_core::hooks_server::CHROME_DEVTOOLS_MCP));
        // The webview opened later takes the port the running agent was told.
        assert_eq!(browser_config_port(&c.to_string()), Some(9333));
        assert!(valid_id("a-1").is_ok());
        assert!(valid_id("a b").is_err());
    }

    #[test]
    fn every_supported_agent_gets_the_browser_server() {
        let (config, role) = (Path::new("c.json"), Path::new("role.md"));
        let (args, env) = wiring(AgentKind::Codex, 9333, config, role, Path::new("."));
        assert_eq!(args[0], "-c");
        assert!(args[1].starts_with("mcp_servers.browser={command='"));
        assert!(args[1].ends_with("'--no-usage-statistics','--browserUrl','http://127.0.0.1:9333']}"));
        assert!(!args[1].contains(' ') && env.is_empty());
        let (args, env) = wiring(AgentKind::OpenCode, 9333, config, role, Path::new("."));
        let content: Value = serde_json::from_str(&env[0].1).unwrap();
        assert!(args.is_empty() && env[0].0 == "OPENCODE_CONFIG_CONTENT");
        assert_eq!(content["mcp"]["browser"]["command"].as_array().unwrap().last().unwrap(), "http://127.0.0.1:9333");
        assert_eq!(content["instructions"][0], "role.md");
        assert_eq!(wiring(AgentKind::Gemini, 9333, config, role, Path::new(".")).1, vec![("AGENTDECK_BROWSER_PORT".into(), "9333".into())]);
        assert_eq!(wiring(AgentKind::Claude, 9333, config, role, Path::new(".")).0[0], "--mcp-config");
    }

    #[test]
    fn mobile_emulation_sets_and_clears_the_user_agent() {
        let [(method, on), (_, touch)] = mobile_emulation(true);
        assert_eq!(method, "Emulation.setUserAgentOverride");
        assert_eq!(on["userAgent"], MOBILE_UA);
        assert_eq!(touch["enabled"], true);
        let [(_, off), _] = mobile_emulation(false);
        assert_eq!(off["userAgent"], "");
    }

    #[test]
    fn urls_default_to_https_and_refuse_other_schemes() {
        assert_eq!(parse_url("example.com").map(|u| u.to_string()), Ok("https://example.com/".into()));
        assert!(parse_url("http://localhost:3000").is_ok());
        assert!(parse_url("file:///c:/x").is_err());
        assert!(parse_url("javascript:alert(1)").is_err());
    }
}
