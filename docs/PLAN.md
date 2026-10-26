# Plan - Multi-agent dashboard (Tauri)

> Working name: **AgentDeck** (may change)
> Goal: a desktop app that opens several terminals side by side, each running an AI agent (Claude Code, Codex, Gemini CLI), with a status bar per agent and a floating voice widget to talk to them directly.

---

## 1. Tech stack

| Layer | Choice | Why |
|---|---|---|
| Desktop app | **Tauri v2** | Light, fast, native multi-window |
| Core (backend) | **Rust** | Terminal management, audio, global shortcuts |
| UI | **React + TypeScript + Vite** | Already familiar with TSX |
| Terminal rendering | **xterm.js** (+ `fit`, `webgl`, `search` addons) | Standard (VS Code uses it) |
| Real terminals | **`portable-pty`** crate | Works on Windows (ConPTY), macOS, Linux |
| Layout | **`dockview`** or **`react-resizable-panels`** | Resizable / movable panes |
| UI state | **Zustand** | Simple, little code |
| Styling | **Tailwind CSS** | Quick way to a clean dark theme |
| Global shortcut | **`tauri-plugin-global-shortcut`** | Key that works even when the app is not focused |
| Audio | **`cpal`** crate (mic capture) | Rust side, no webview permission issues |
| Transcription | **`whisper-rs`** (local whisper.cpp) or an API (Whisper / Deepgram) | Local = free and private; API = faster on a small PC |
| Persistence | **`tauri-plugin-store`** | Save the layout and sessions |

---

## 2. Features

### 2.1 Terminal grid
- Main screen = grid of panes, each one a terminal running an agent.
- **"+ Add agent"** button opens a picker:
  - Agent choice: **Claude Code**, **Codex**, **Gemini CLI** (cards with logo, name, "installed / not found" status).
  - Working folder.
  - Option: create a dedicated **Git worktree** (so agents do not step on each other).
  - Optional arguments (model, permission mode...).
- Resizable, movable panes; maximize a pane (double-click the header); close.
- **Detach a pane into a separate window** (Tauri multi-window), handy with two screens.
- Tabs at the top for several "sessions" (for example one project per tab).

### 2.2 Agent logos
- Put the logos in `src/assets/agents/` (SVG).
- Use the official logos from each brand's press kit and follow their usage rules (no changes, minimum size, etc.).
- Each agent also gets an **accent color** (pane border, dot) to recognize it at a glance.

### 2.3 Info bar above each terminal
Pane header content:
- Logo + agent name + task title (editable)
- **Status** with a colored dot:
  - 🟢 done · 🔵 working · 🟠 waiting for your answer / a permission · 🔴 error · ⚪ idle
- Last action (for example "Edit `src/STYLE.md`", "Run a shell command")
- Time spent on the current task
- Finished task counter
- Buttons: restart, maximize, detach, close

**How to know what the agent is doing (most to least reliable):**
1. **Official hooks**: the app runs a small local HTTP server (for example `localhost:47800`) and configures the agent's hooks to send events to it:
   - Claude Code: `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Notification`, `Stop` hooks (`http` type or a `curl` command).
   - Codex: `notify` option / hooks in `~/.codex/config.toml`.
   - Gemini CLI: check which hook system is available at dev time.
   - Each pane has an id passed in an environment variable (`AGENTDECK_PANE_ID`) to know which pane sent the event.
2. **Terminal output parsing** (spot "Worked for...", permission prompts, etc.) as a fallback.
3. **Idle detection**: no output for X seconds means "idle / waiting".

**Notifications**: system notification + sound when an agent becomes "done" or "waiting for your answer" (`tauri-plugin-notification`).

### 2.4 Voice widget (floating pill)
- A **separate Tauri window**: transparent, borderless, **always on top**, centered at the bottom of the screen (movable).
- Shape: **rounded black pill** with bars that move with the mic volume.
- Visible **above all windows**, even when the app is in the background.

**How it works:**
1. Global shortcut (for example `Ctrl+Space` or a configurable key).
2. Mode of choice: **hold to talk** (push-to-talk) or **press / press again**.
3. While listening: the pill shows up and the bars move.
4. On release: transcription, then the text is **typed into the active agent's terminal** (the selected / focused one).
5. Option: preview the text in the pill before sending (Enter to confirm, Esc to cancel).
6. Bonus option: start the sentence with the agent's name to target it ("Codex, run the tests").

**Technical details:**
- Audio capture with `cpal` on the Rust side, resampled to 16 kHz mono for Whisper.
- The sound level (RMS) is sent to the pill ~30 times/s through a Tauri event to animate the bars.
- Write into the active pane's PTY + `\r` to submit (configurable: send or just type).

---

## 3. Architecture

```
agent-deck/
├── src-tauri/                 # Rust backend
│   ├── src/
│   │   ├── main.rs
│   │   ├── pty/               # Terminal creation/management (portable-pty)
│   │   ├── agents/            # Claude / Codex / Gemini definitions, install detection
│   │   ├── hooks_server/      # Local HTTP server that receives agent events
│   │   ├── voice/             # Mic capture (cpal) + transcription (whisper-rs or API)
│   │   ├── shortcuts.rs       # Global shortcuts
│   │   └── windows.rs         # Main window, detached windows, voice pill
│   └── tauri.conf.json
├── src/                       # React UI
│   ├── app/                   # Main layout, session tabs
│   ├── components/
│   │   ├── TerminalPane/      # xterm.js + info header
│   │   ├── AgentPicker/       # Agent picker with logos
│   │   ├── StatusBadge/
│   │   └── VoicePill/         # The animated pill
│   ├── store/                 # Zustand: panes, statuses, settings
│   ├── assets/agents/         # SVG logos
│   └── voice.html             # Entry point of the pill window
└── package.json
```

**Frontend to Rust communication (Tauri commands and events):**
- `spawn_agent(agent, cwd, args) -> pane_id`
- `write_to_pane(pane_id, data)` / `resize_pane(pane_id, cols, rows)` / `kill_pane(pane_id)`
- `pane-output:{id}` event: terminal data to xterm.js
- `agent-status` event: header update
- `voice-level`, `voice-transcript`, `voice-state` events

---

## 4. Development steps

### Step 1 - Base (1-2 days)
- [ ] Create the project: `npm create tauri-app@latest` (React + TS)
- [ ] Add Tailwind, Zustand, xterm.js
- [ ] A single terminal that runs a shell through `portable-pty` and shows in xterm.js
- [ ] Handle resizing (`fit` addon + `resize_pane`)

### Step 2 - Multiple terminals + agent picker (2-3 days)
- [ ] Grid of resizable panes (dockview)
- [ ] "Add agent" window with the 3 cards and logos
- [ ] Detection: are `claude`, `codex`, `gemini` installed? (link to the install page otherwise)
- [ ] Folder choice + Git worktree option
- [ ] Save/restore the layout

### Step 3 - Info bar and statuses (2-3 days)
- [ ] Local HTTP server for hooks
- [ ] Automatic Claude Code hook setup (in the project settings, without overwriting existing ones)
- [ ] Same for Codex, then Gemini
- [ ] Header with dot, last action, timer, counter
- [ ] System notifications + sound

### Step 4 - Voice widget (3-4 days)
- [ ] Transparent, always-on-top pill window
- [ ] Global push-to-talk shortcut
- [ ] Mic capture + bar animation
- [ ] Transcription (start with an API to go fast, then local whisper)
- [ ] Send the text to the active pane

### Step 5 - Polish
- [ ] Panes that detach into separate windows
- [ ] Themes (dark by default)
- [ ] Settings page (shortcuts, Whisper model, send mode)
- [ ] Windows / macOS installers (`tauri build`)

---

## 5. Things to watch

- **Windows**: `portable-pty` uses ConPTY, which needs Windows 10 1809+. Test colors and resizing well.
- **macOS**: declare `NSMicrophoneUsageDescription` in `Info.plist`, otherwise the mic is blocked.
- **Linux**: transparent windows depend on the window manager.
- **Performance**: with many panes, batch PTY output (send every ~16 ms rather than on every byte) and enable the xterm.js WebGL addon.
- **Hooks**: never overwrite the user's existing config; merge cleanly and support uninstalling.
- **Local Whisper**: the `base` or `small` model is enough for commands; `large` is slow without a GPU.
- **Security**: the hook server only listens on `127.0.0.1`, with a secret token.

---

## 6. Ideas for later
- An "orchestrator" agent that splits tasks between the others
- Session history with search
- Cost / quota used per agent
- Text to speech: the agent reads you a summary when it is done
- "Wall" mode: compact view of 6-9 agents with just their statuses
