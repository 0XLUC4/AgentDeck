# AgentDeck GPUI port

Pure Rust rewrite of AgentDeck (`D:\agentdeck`, Tauri + React) with GPUI. The original is read only.

- `crates/core` (`agentdeck-core`): everything that is not UI. The Tauri backend (`src-tauri/src`)
  copied and de-Tauri-fied, plus the pure TypeScript logic of `src/lib` and `src/store` in `core::logic`.
  `cargo test -p agentdeck-core` runs the backend tests and the ported vitest suites (172 tests).
- `crates/app` (`agentdeck` binary): the GPUI UI, one module per feature area.

Toolchain: `rust-toolchain.toml` pins Rust 1.99.1 (gpui-pre 0.3.11 needs `std::hint::cold_path`).
UI stack: `gpui-kit = "0.9"` from crates.io. It pins the matching GPUI snapshot (`gpui-pre 0.3.11`) and
re-exports GPUI (`gpui_kit::*`), gpui-component (`gpui_kit::component`) and Lucide icons
(`gpui_kit::assets`). Do not add `gpui` from git next to it: two GPUI versions will not link together.
Windows is supported (DirectX renderer).

## Rules for every agent

- Edit only the files you own (table below). Need something from another module? Use its public
  interface listed here. If it is missing, add a small private shim in your own files and report it.
- Real error handling: no `unwrap`/`expect` on fallible IO in app paths; failures become a toast
  (`Deck::toast`) or an `eprintln!` for background work.
- Colors only from `theme::palette(cx)` (index.css tokens), never hard-coded hex in views.
- Labels, shortcuts and behavior match the original. No em dashes in code comments or UI text.
- `cargo check --workspace` must stay green. Do not commit.
- Deps: the app `Cargo.toml` already has what the feature areas need (below). Ask before adding more.

## Module ownership

| Module | Owner scope | Original files |
|---|---|---|
| `core/*` | backend + logic (foundation agent) | `src-tauri/src/*`, `src/lib/*.ts`, `src/store/*.ts` |
| `app/src/main.rs`, `state.rs`, `theme.rs`, `Cargo.toml` | foundation agent | `src/main.tsx`, `src/App.tsx`, `src/index.css` |
| `app/src/terminal/` | terminal emulator view | `components/TerminalPane/TerminalPane.tsx`, `lib/{ringBuffer,wakeTerminal,fileRefs}.ts` |
| `app/src/layout/` | window shell, splits, tabs, detach | `app/{TitleBar,Sidebar,TabBar,Deck,Activity,TaskDetail,Toasts}.tsx`, `components/Detached/DetachedPane.tsx`, `pane.tsx`, `lib/{gridLayout,detach}.ts` |
| `app/src/panes/` | pane chrome + pane lifecycle | `components/TerminalPane/{PaneHeader,PaneMenu,PanePanel,WorkingOutline,VoiceGlow}.tsx`, `components/{AgentPicker,AgentLogo,StatusBadge,Permission,IconButton}`, `store/runtime.ts` |
| `app/src/palette/` | command palette, broadcast, queue, templates picker | `components/{CommandPalette,Broadcast,Queue,Templates/TemplatePicker}`, `lib/recentPrompts.ts` |
| `app/src/git/` | git panel, diff chip | `components/Git/{GitPanel,DiffChip}.tsx`, `lib/checkpoints.ts` |
| `app/src/history/` | history, journal, usage, budget | `components/{History,Journal,Usage}`, `lib/budget.ts` |
| `app/src/settings/` | settings dialog, all sections | `components/Settings/*`, `components/Templates/TemplatesSection.tsx`, `components/History/HistorySettings.tsx` |
| `app/src/skills/` | skills, notes, agent dialog, file editor, graph, assets | `components/{Skills,Graph,Assets}` |
| `app/src/browser/` | browser panel (only web view) | `components/Browser/BrowserPanel.tsx`, `src-tauri/src/browser.rs` |
| `app/src/orchestrator/` | orchestrator, wall, link workspace, trust | `components/{Orchestrator,Wall,LinkWorkspace}`, `lib/orchestrator.ts` |
| `app/src/voice/` | voice pill window, dictation routing, TTS | `components/VoicePill`, `voice.tsx`, `lib/{voiceListener,voiceRoute,tts}.ts` |
| `app/src/system/` | tray, notifications, updater, autostart, global shortcut, single instance | `src-tauri/src/{tray,notify}.rs`, `lib/{notify,tray,updater}.ts`, `components/Update`, `components/Settings/AppControls.tsx` |

Each feature module may split its `mod.rs` into more files inside its own folder.

## Feature inventory (original -> new)

### Backend (`src-tauri/src` -> `core`)
| Original | New | Notes |
|---|---|---|
| `lib.rs` (setup, AppState, commands) | `core::lib` (`start`, `AppHandle`, `AppState`, `shutdown`) | Tauri commands are plain pub fns |
| `pty/mod.rs` | `core::pty` | sinks are `std::sync::mpsc::Sender<String>` instead of IPC channels |
| `agents/mod.rs` | `core::agents` | detect, hooks install/uninstall, worktrees |
| `hooks_server/{mod,permissions}.rs` | `core::hooks_server` | HTTP 127.0.0.1:47800, emits bus events |
| `mcp/{mod,rpc,status,hub}.rs` | `core::mcp` | orchestrator MCP + shared MCP hub |
| `git/mod.rs` | `core::git` | |
| `history/{mod,ansi,journal}.rs` | `core::history` | `write_text_file` takes the path; the save dialog is the UI's (rfd) |
| `usage/{mod,parse,prices}.rs` | `core::usage` | |
| `voice/{mod,audio,config,local}.rs` | `core::voice` | global shortcut registration and pill window moved to the app |
| `workspace/mod.rs` | `core::workspace` | open projects (VS Code, Cursor, Explorer...), inspect folder |
| `skills.rs` | `core::skills` | `save_paste_image(bytes, ext)` |
| `assets.rs`, `notes_graph.rs` | `core::assets`, `core::notes_graph` | no asset-protocol scope needed |
| `trust.rs`, `notify.rs` | `core::trust`, `core::notify` | |
| `tray.rs` | `app::system` | tray-icon crate |
| `browser.rs` | `app::browser` | gpui-webview (Wry) |
| tauri-plugin-store | `core::persist` | same files and keys, see below |

### Pure TS logic (`src/lib`, `src/store` -> `core::logic`)
| Original | New |
|---|---|
| `store/deck.ts` | `logic::deck` (`DeckState`, `Pane`, `PaneSpec`, `Workspace`, `Tab`, `SavedSession`, `apply_status`, `session_status`, `activity_key`, ...) |
| `store/settings.ts` | `logic::settings` (`Settings`, `merge_settings`) |
| `store/persist.ts` | `core::persist` (`migrate`, `hydrate`, `plan_restore`, `specs_changed`, `load_*`/`save_*`) |
| `store/queue.ts`, `store/review.ts`, `components/Broadcast/send.ts`, `lib/recentPrompts.ts` | `logic::queue` |
| `store/tasks.ts` | `logic::tasks` |
| `store/templates.ts` | `logic::templates` |
| `lib/orchestrator.ts` (pure parts + read-only tools) | `logic::orchestrator` |
| `components/Permission/autoAllow.ts` | `logic::permissions` |
| `lib/{splitArgs,fileRefs,shortAction,shortcut,tts(text),clock,agents,budget(overLimit)}.ts`, `components/Usage/format.ts`, `components/Wall/wallState.ts` (lastLines) | `logic::text` |
| `lib/ringBuffer.ts` | `logic::ring_buffer` |
| `lib/voiceRoute.ts` | `logic::voice_route` |
| `lib/theme.ts`, `index.css` tokens | `logic::theme` (`Palette`, OKLCH -> hex, presets) |
| `lib/gridLayout.ts`, `lib/detach.ts` | `logic::layout` (`grid_rows`, `split_sizes`, `ReplayGate`, `forwards_input`) |
| `lib/ipc.ts`, `lib/types.ts` | replaced by direct calls into `core` and its serde types |
| `store/runtime.ts` | `app::panes` (pane lifecycle needs the UI: terminals, timers) |

### UI features (each must reach parity)
- Window: custom title bar (sidebar toggle, broadcast, interrupt all, wall, activity, usage chip, update, settings),
  min 800x500, default 1400x900, dark background `#131313` while loading.
- Sidebar: workspaces (link folder, rename, unlink), sessions per workspace (open, archived, reopen, delete),
  session status dots.
- Tab bar: sessions of the active workspace, rename (F2 / double click), new, archive, menu.
- Pane area: splits and tabs (dock), auto grid (`settings.auto_grid`), maximize (Esc restores), drag to rearrange,
  detach a pane into its own window and reattach, Ctrl+1..9 focus Nth pane.
- Pane header: agent logo + color tag, editable title, status badge (waiting pulses), task timer, short last
  action (tooltip: full), diff chip, queue button, usage, interrupt, restart, close, menu (zoom, color, browser,
  review by, detach, git, skills...). Working outline sweep and voice glow.
- Terminal: full VT emulation, 512K scrollback replay, copy/paste, image paste and file drop as agent refs,
  zoom Ctrl+= / Ctrl+- / Ctrl+0, search.
- Permission cards: allow, always allow, deny with message, ask in terminal, auto-allow here.
- Activity panel (Ctrl+Shift+A), next waiting agent (Ctrl+Shift+J), task detail (prompt, actions, summary).
- Command palette (Ctrl+K or Ctrl+Shift+P), broadcast (Ctrl+Shift+B, Ctrl+Enter sends), prompt queue
  (Ctrl+Enter adds), templates (builtin + saved).
- Git panel, checkpoints before each task, compare two panes, merge worktree, PR, pull/push, clone.
- History search (Ctrl+Shift+F), journal of the day (Markdown export), usage dialog, budget limits.
- Settings (Ctrl+,): General, Appearance, Notifications, Voice, Agents, MCP hub, Templates, About.
- Skills / notes / agent files (Ctrl+S saves) / notes graph / 3D assets viewer.
- Orchestrator pane + activity log + pause, wall (Ctrl+Shift+W, Esc exits), link workspace dialog.
- Voice: global push-to-talk shortcut, pill window, routing by spoken agent name or pane title, TTS summaries.
- System: tray (Show AgentDeck / Quit, close to tray), notifications (sound, phone push via ntfy), taskbar
  flash, stuck / remind / rate-limit retry watchers, autostart, updater, single instance.

## Public interfaces

### `agentdeck_core` (crate root)
```rust
pub fn start(data_dir: PathBuf) -> AppHandle;      // hook server, MCP hub, AppState, voice, history
pub fn shutdown();                                  // app exit: stops hub servers
pub struct AppHandle;                               // Clone
impl AppHandle {
    pub fn default_data_dir() -> Result<PathBuf, String>;   // %APPDATA%/com.agentdeck.app (same as Tauri)
    pub fn app_data_dir(&self) -> Result<PathBuf, String>;
    pub fn emit<S: Serialize>(&self, name: &str, payload: S) -> Result<(), String>;
    pub fn subscribe(&self) -> std::sync::mpsc::Receiver<Event>;
    pub fn manage<T>(&self, value: T);  pub fn state<T>(&self) -> Arc<T>;  pub fn try_state<T>(&self) -> Option<Arc<T>>;
}
pub struct Event { pub name: String, pub payload: serde_json::Value }
pub struct AppState { pub panes: pty::Panes, pub active_pane: Mutex<Option<String>>, pub hooks: HookServer }
```

### Bus events (same names and JSON as the Tauri events)
| Event | Payload |
|---|---|
| `agent-status` | `hooks_server::AgentStatusEvent` (camelCase) |
| `agent-permission` | `hooks_server::permissions::PermissionRequest` |
| `agent-permission-resolved` | `{ requestId }` |
| `mcp-request` | `{ requestId, orchestratorId, tool, args }` (answer with `mcp::mcp_respond`) |
| `mcp-activity` | `{ orchestratorId, tool, args, error }` |
| `pane-exit:<paneId>` | `null` |
| `voice-level` | f32 0..1 |
| `voice-state` | `"idle" \| "listening" \| "downloading" \| "transcribing" \| "error"` |
| `voice-download` | `{ model, received, total }` |
| `voice-partial`, `voice-transcript`, `voice-error` | string |
| `voice-pill` | `"show" \| "hide"` (new: replaces the Tauri pill window calls) |

`app::state::Deck` already consumes `agent-status`, `agent-permission(-resolved)`, `mcp-activity`,
`pane-exit:*` and the read-only `mcp-request` tools, then re-emits every event as `DeckEvent::Backend(Event)`.

### Backend functions (blocking: call from `cx.background_spawn`, not the UI thread, when they do IO)
```rust
// pty
pty::spawn_pane(app: &AppHandle, pane_id: String, agent: AgentKind, cwd: String, args: Vec<String>, worktree: bool,
                cols: u16, rows: u16, title: Option<String>, env: Option<HashMap<String,String>>, trust: Option<bool>) -> Result<(), String>;
pty::attach_pane_output(owner: &str, pane_id: String, on_output: mpsc::Sender<String>) -> u64;
pty::detach_pane_output(pane_id: String, sink: u64);   pty::drop_owner_sinks(owner: &str);
pty::write_to_pane(state: &AppState, pane_id: String, data: String) -> Result<(), String>;   // or state.panes.write(id, bytes)
pty::resize_pane(state: &AppState, pane_id: String, cols: u16, rows: u16) -> Result<(), String>;
pty::kill_pane(state: &AppState, pane_id: String) -> Result<(), String>;
pty::list_panes(state: &AppState) -> Result<Vec<String>, String>;
pty::set_active_pane(state: &AppState, pane_id: Option<String>) -> Result<(), String>;
// agents
agents::detect_agents() -> Vec<AgentInfo>;   agents::uninstall_hooks(cwd: String) -> Result<(), String>;
// permissions
hooks_server::permissions::answer_permission(request_id: String, decision: String /*allow|allow_always|deny|ask*/, message: Option<String>) -> Result<(), String>;
// mcp
mcp::mcp_register(app: &AppHandle, state: &AppState, pane_id: String) -> Result<Registration /*args, secret*/, String>;
mcp::mcp_respond(request_id: String, result: Option<Value>, error: Option<String>) -> Result<(), String>;
mcp::mcp_set_paused(paused: bool);
mcp::hub::{hub_list() -> Vec<HubServer>, hub_save(Vec<HubServer>) -> Result<(),String>, hub_test(name: String) -> Result<usize,String>, hub_claude_servers() -> Vec<HubServer>};
// git (cwd: String everywhere)
git::{git_status, git_diff(cwd, path), git_diff_trees(cwd, from, to, path), git_commit(cwd, message), git_merge_worktree,
      git_create_pr(cwd, title, body), git_remove_worktree, git_pane_dir(cwd, pane_id, worktree) -> String,
      git_checkpoint(cwd, pane_id) -> Result<Option<String>>, git_list_checkpoints(cwd, pane_id), git_restore_checkpoint(cwd, pane_id, sha),
      git_compare(cwd, other_cwd), git_upstream, git_pull, git_push, gh_list_repos() -> Result<Value>, git_clone(repo, parent)};
// voice
voice::{voice_start(&AppHandle), voice_stop(&AppHandle), voice_get_config(&AppHandle) -> Result<VoiceConfigView>,
        voice_set_config(&AppHandle, VoiceConfig) -> Result<VoiceConfigView>, voice_set_api_key(Option<String>),
        voice_list_models(&AppHandle) -> Result<Vec<ModelStatus>>, voice_download_model(&AppHandle, WhisperModel) /*blocking*/,
        on_shortcut(&AppHandle, pressed: bool)};
voice::Shortcut = global_hotkey::hotkey::HotKey (parse the config string with `.parse()`);
// workspace, notify, usage, history, journal
workspace::{list_open_projects() -> Vec<OpenProject>, inspect_folder(path: String) -> Result<FolderInfo>};
notify::push_notify(url: String, title: String, body: String) -> Result<(), String>;
usage::{usage_summary(range: String /*today|7d|30d*/) -> Result<UsageSummary>, pane_usage(pane_id, cwd, started_at: i64, agent: String) -> Result<PaneUsage>};
history::{history_search(query, limit) -> Vec<SearchHit>, history_read(run_id, around_line) -> Result<RunWindow>,
          history_get_enabled() -> bool, history_set_enabled(bool) -> Result<()>, history_clear() -> Result<()>};
history::journal::{journal_append(Value), journal_read(date: String) -> Result<Vec<Value>>, journal_git_commits(cwd) -> Result<Vec<Commit>>,
                   write_text_file(path: &Path, contents: &str) -> Result<()>};
// skills, assets, notes
skills::{skills_list(agent, cwd), skills_install(agent, cwd, url, scope), skills_remove(agent, cwd, path), notes_list(agent, cwd),
         agent_file_read(agent, cwd, path), agent_file_write(agent, cwd, path, text), save_paste_image(bytes: &[u8], ext: &str) -> Result<String>};
assets::{assets_list(cwd) -> Result<Vec<AssetFile>>, asset_source(cwd, path) -> Result<String>};
notes_graph::{notes_graph(cwd) -> Result<NotesGraph>, note_read(cwd, id) -> Result<String>};
trust::trust_folder(agent, cwd: &str);
```

### `core::logic` (pure, tested)
```rust
deck:   DeckState { workspaces, panes: HashMap<String,Pane>, tabs, active_tab_id, active_pane_id, archived, last_cwd, toasts }
        methods: active_tab, active_workspace, workspace_for_path, activity_groups, update_pane, add_pane, remove_pane,
        set_status -> bool, acknowledge, rename, zoom(Option<f32>), add_tab, remove_tab, archive_tab -> Vec<pane ids>,
        unarchive -> Option<(tab id, Vec<PaneSpec>)>, delete_archived, rename_tab, set_tab_layout(Value), link_workspace,
        rename_workspace, select_workspace, unlink_workspace -> Vec<pane ids>, toast, prune_toasts
        Pane derefs to PaneSpec; Layout = Option<serde_json::Value> (opaque, owned by app::layout)
        fns: new_pane, apply_status, clamp_font_size, session_status, activity_key, can_archive, folder_name, new_id
settings: Settings (+ Default), merge_settings(&Value), Settings::agent(kind)
queue:  Queues { add, remove, move_by, set_paused, forget, pop_next, requeue_front }, prompt_text, should_advance,
        ready_for_next, move_item, creates_loop, review_prompt, remember_prompt, ADVANCE_DELAY_MS, SUBMIT_DELAY_MS
tasks:  TaskLog, Task, TaskEvent::from(&AgentStatusEvent), apply_task_event(log, ev, now, fallback_start)
templates: SessionTemplate, builtin_templates, all_templates, session_to_template, instantiate, relative_path, join_path, parse_templates
orchestrator: OrchestratorState { paused, log; record }, summarize_call, resolve_inside, scope_of, target, assert_can_type, handle_read_tool
permissions: auto_allowed, rule_matches, rule_for, parse_rule, inside_folder
text:   split_args, file_refs, image_ext, interrupt_key, short_action, KeyCombo + to_shortcut, format_shortcut, clean_for_speech,
        speech_text, format_elapsed, format_since, format_tokens, format_cost, last_lines, over_limit, agent_name, AGENT_KINDS, now_ms
ring_buffer: RingBuffer { push, text, tail, clear }
voice_route: route_transcript(text, &[RouteCandidate]) -> Route { pane_id, text }, agent_aliases
theme:  Palette::{dark, light, for_appearance(&Appearance, system_light)}, Palette::agent(kind), oklch, parse_hex, luminance,
        on_accent, mix, ACCENT_PRESETS, BACKGROUND_PRESETS, TERMINAL_* colors, FONT_SANS, FONT_MONO, RADIUS_*
layout: grid_rows, split_sizes, ReplayGate { output, replay }, forwards_input
```

### `core::persist` (app data dir, same as the Tauri build)
`store.json` key `deck` (`Saved` v2), `settings.json` key `settings`, `templates.json` key `templates`,
`local.json` for what the web build kept in localStorage. `JsonStore::{load, get, set, save}` (atomic write),
`load_deck(dir, &mut DeckState) -> Vec<SavedSession>`, `save_deck(dir, &DeckState, now)`, `load_settings`,
`save_settings`, `load_templates`, `save_templates`, `migrate`, `hydrate`, `plan_restore`, `specs_changed`,
`SAVE_DEBOUNCE_MS = 300`.

### App crate
```rust
// theme.rs
pub struct AppTheme(pub Palette);            // GPUI global
pub fn palette(cx: &App) -> Palette;          // read colors in render
pub fn hsla(hex: u32) -> Hsla;  pub fn hsla_a(hex: u32, alpha: f32) -> Hsla;
pub fn apply(appearance: &Appearance, window: Option<&mut Window>, cx: &mut App);   // also restyles gpui-component

// state.rs
pub struct Deck { pub backend: AppHandle, pub data_dir: PathBuf, pub state: DeckState, pub settings: Settings,
                  pub queues: Queues, pub tasks: HashMap<String, TaskLog>, pub reviews: HashMap<String, String>,
                  pub orchestrator: OrchestratorState, pub templates: Vec<SessionTemplate>,
                  pub permissions: Vec<PermissionRequest>, pub recent_prompts: Vec<String>,
                  pub pending_restore: Vec<SavedSession> }
impl EventEmitter<DeckEvent> for Deck;  pub enum DeckEvent { Backend(Event) }
impl Deck { pub fn save(&mut self, cx); pub fn update_settings(&mut self, edit: impl FnOnce(&mut Settings), cx);
            pub fn toast(&mut self, message, cx); }

// layout: DeckView::new(deck, window, cx), deck(), terminal(pane_id, cx); free fns reveal_pane, jump_to_pane,
//         toggle_maximize_pane, arrange_in_grid, toggle_detached, is_detached
// terminal::TerminalView::new(pane_id) (+ pub deck, forwards_input), Focusable
// panes: runtime::{start, create_pane, send_input, send_prompt, focus_pane, restart_pane, close_pane, ...},
//        PaneHeader (+ PaneHeaderEvent, pub extras), pane_frame, AgentPicker, PermissionBanner, agent_logo
// browser::{open, go, url, close}, BrowserPanel::new(deck, pane_id, window, cx) (emits BrowserRemoved)
// git::{DiffChip, GitPanel, checkpoint_pane}; history::{UsageChip, overlay, OpenUsage, SearchHistory, OpenJournal}
// palette::{queue_button, remember_prompt, open_broadcast, open_template_picker_for}
// orchestrator::{wall_overlay, orchestrator_badge, open_link_workspace, create_orchestrator, set_paused}
// skills::{overlay, open_skills, open_notes, open_assets, open_graph}; settings::OpenSettings
// system::{single_instance, main_window, main_deck, notify, UpdateButton, AppControls, set_voice_shortcut}
// voice::{voice_activity, voice_target}
// every feature module: pub fn init(cx: &mut App), called once from main
```

### Actions and key bindings already registered
| Action (namespace `agentdeck`) | Keys | Module |
|---|---|---|
| `palette::OpenPalette`, `palette::OpenPaletteAlt` | ctrl-k, ctrl-shift-p | palette |
| `palette::OpenBroadcast` | ctrl-shift-b | palette |
| `history::SearchHistory` | ctrl-shift-f | history |
| `settings::OpenSettings` | ctrl-, | settings |
| `orchestrator::ToggleWall` | ctrl-shift-w | orchestrator |

Also bound: ctrl-1..9, ctrl-shift-a (activity), ctrl-shift-j (next waiting), F2 (rename session) in `layout`;
ctrl-= / ctrl-- / ctrl-0 zoom, ctrl-v paste, ctrl-c / ctrl-shift-c copy in the terminal key handler; ctrl-s in the
`skills` editor. `layout` also has global fallbacks for `ToggleSidebar`, `ToggleActivity`, `NextWaiting` and
`AddAgent` (the palette dispatches them from its dialog, where DeckView's own listeners are not on the path).

### Dependencies available in `crates/app`
`gpui-kit 0.9` (GPUI + gpui-component + Lucide icons), `smol 2`, `alacritty_terminal 0.26` (terminal),
`gpui-webview 0.9` (browser), `global-hotkey 0.8` (same `HotKey` type as core), `tray-icon 0.26`,
`notify-rust 4`, `auto-launch 0.6`, `rfd 0.17` (native file dialogs), `arboard 3` (clipboard images), `image 0.25`.

## Integration status (2026-11-07)

`cargo build --release -p agentdeck` and `cargo test --workspace` pass (app 51 tests, core 172).
`main.rs` loads `AllAssets` (full Lucide set), calls `layout::init` and `panes::start` (session restore,
1 s clock, autosave, exit and hook handlers). Test modules import `core::prelude::v1::test` after
`use super::*`: the `gpui_kit::*` glob brings GPUI's own `test` macro, which recurses on `#[test]`.

Wiring done in the integration pass:
- Pane header: `git::DiffChip` and `history::UsageChip` in `extras`, `palette::queue_button`, `orchestrator::orchestrator_badge`
  (crown + activity log). The pane frame carries the `pane` hover group the queue button needs.
- Browser: `panes::runtime` opens the webview in the main window before a Claude spawn with `browser` on and
  passes its `--mcp-config` args; `close_pane` closes the webview.
- Jumps (Wall, permission card "Terminal", Ctrl+1..9, palette, activity) go through `layout::jump_to_pane`: the
  pane is revealed in its dock (leaves maximize) and keyboard focus moves into its terminal.
- Voice glow and header mic read the target and "sent" flash from `voice::{voice_target, voice_activity}`.
- Prompts submitted through hooks go through `palette::remember_prompt` (saved in local.json).

| Feature | Status |
|---|---|
| Window shell, title bar, sidebar, tab bar, toasts | ported, wired; launched (see below) |
| Dock: splits, tabs, drag, sash, maximize, auto grid, saved dockview layouts | ported; not tried by hand |
| Detached pane windows | ported; not tried by hand |
| Terminal (alacritty_terminal grid, GPU drawing, input, IME composition, paste, drop, mouse, zoom) | ported (the original loads no search addon either) |
| Pane header, menu, working outline, voice glow, exit bar | ported, wired |
| Pane runtime (spawn queue, restore, exit, hooks, rate-limit retry) | ported, started from main |
| Permission cards and banner | ported, wired |
| Activity panel + TaskDetail (prompt, actions, result, copy, send result to, previous tasks) | ported (`layout/activity.rs`) |
| Command palette, broadcast, queue, templates | ported, wired |
| Git panel, diff chip, checkpoints, code-then-review runner | ported, wired (review runner in `palette/queue.rs`) |
| History search, journal, usage, budget watch | ported, wired |
| Settings (all sections) | ported, Ctrl+, |
| Skills, notes, file editor, notes graph, assets, 3D preview | ported; CPU renderer (`skills/model3d.rs`): glTF/GLB/.blend, OBJ+MTL, STL with orbit, zoom, part picking and tile thumbnails; flat colors (textures averaged), no FBX |
| Browser panel | ported, wired into spawn and close |
| Orchestrator tools, wall, link workspace, activity log | ported, wired |
| Voice: shortcut, pill window, routing, TTS | ported |
| System: tray, close to tray, notifications, watchers, updater, autostart, single instance | ported |

Sidebar: workspace rename (pencil or double click), actions on hover for inactive workspaces, archived
sessions with reopen / delete (confirm) and the archive date.

Known gaps: FBX preview and textured 3D rendering, the updater still reading the Tauri feed (this crate is
0.2.0, the Tauri app 0.1.20, and the feed's installer is the Tauri app), and the UI has only been launched,
not clicked through (dialogs, drag and drop, detach, browser, voice untested on screen).
