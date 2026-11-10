<p align="center">
  <img src="crates/app/assets/logo.png" width="96" alt="AgentDeck logo" />
</p>

<h1 align="center">AgentDeck</h1>

<p align="center">
  A native desktop dashboard to run Claude Code, Codex, Gemini CLI, Grok Build and OpenCode side by side,
  see what each one is doing, and talk to them with your voice.
</p>

<p align="center">
  <a href="https://github.com/0XLUC4/AgentDeck/releases/latest">Download for Windows</a>
</p>

## Why

Running several coding agents at once usually means juggling terminal windows and checking each one to
see if it's done or waiting for you. AgentDeck puts them all in one window, arranges them in a grid and
tells you which one needs you.

Written in pure Rust with [GPUI](https://www.gpui.rs) and gpui-component: every terminal is rendered on the GPU,
starts instantly and stays light on memory.

## Features

**Agents and terminals**
- Real terminals (ConPTY on Windows, alacritty_terminal emulation) for Claude Code, Codex, Gemini CLI,
  Grok Build, OpenCode or a plain shell
- Panes arrange themselves in a grid (2 side by side, 3 as 2 + 1, 4 as 2x2, 6 as 3x2...), with splits and tabs
- Zoom, maximize, detach a pane into its own window, restart, interrupt
- Optional git worktree per agent so they don't step on each other
- Claude Code agents pick up their conversation again after a restart or when a session is reopened
- Each agent can get its own `PORT` so parallel dev servers don't collide

**Know what's going on**
- Live status for every agent (working, waiting for you, done, error) through the agents' own hooks
  (Claude Code, Codex and Gemini CLI), with a light running around the pane while it works
- Alerts when an agent looks stuck, reminders when one keeps waiting for you, and a warning when two agents
  edit the same file
- Uncommitted changes of each agent (+12 -3 · 4 files) right in its header
- Activity panel across all workspaces, with the prompt, the actions taken and the final answer of each task
- Answer permission prompts and agent questions from the app without switching windows, or let AgentDeck
  auto-allow the ones you trust in a folder ("Bash(pnpm test *)", chained commands never match)
- Desktop notifications with a soft chime when an agent finishes or needs you, also on your phone
  (ntfy or any webhook)
- Daily and per-agent cost limits that warn you, and can stop the agents
- Token and cost estimates per agent, terminal history with search, a daily journal you can export
- Claude subagents shown per pane: how many run, what each one is doing
- Plan under the pane header: steps done out of total, one colored dot per step
- Subject of the Claude conversation next to the agent name, with a status pill (Working, Done, Waiting, Error)
- Notes page per workspace: graph of links or list, with a Markdown reader; Assets opens as a page too
- Find in a terminal's scrollback (Ctrl+Shift+F)

**Workspaces and sessions**
- Link a folder, a git repo or a project already open in VS Code / Cursor / Explorer
- Several sessions per workspace, archived and reopened with their layout
- Race: the same prompt to 2 to 4 fresh agents, one worktree each, then keep the best result
- A setup command run once in each new agent worktree (install dependencies...)
- Session templates, a prompt queue per agent, broadcast one prompt to many agents, send one agent's
  result to another
- Git panel: diff, commit, merge a worktree back or open a pull request (GitHub CLI), resolve merge conflicts
  with an agent, compare two agents, restore points taken before each task
- Auto review: when an agent finishes, another one reviews its changes

**Voice**
- Push-to-talk (Ctrl+Space): a small pill with live mic bars shows at the bottom of the terminal you are talking to
- Offline transcription with whisper.cpp, or any OpenAI-compatible API (OpenAI, Groq...)
- Say "Codex, run the tests" to send it to a specific agent
- Finished tasks can be read aloud when you are not looking at them

**Skills, notes and tools**
- Browse, edit and install agent skills (paste a GitHub link), notes with a graph view, project assets
  with a 3D model viewer
- Built-in browser panel
- Shared MCP hub for all agents

**And**
- Command palette (Ctrl+K), wall view of every agent, orchestrator agent that can drive the others over MCP
  (spawn, prompt, wait, read diffs, undo a task, merge worktrees, close agents)
- System tray, launch at startup, signed auto-updates (Update and restart)

## Install

Download `agentdeck.exe` from the [releases page](https://github.com/0XLUC4/AgentDeck/releases/latest) and start it.
It is a single portable exe, no installer and no admin rights needed. Windows SmartScreen may warn the first time
because the exe isn't code-signed yet: click "More info" then "Run anyway".

You need at least one agent CLI on your PATH:
[Claude Code](https://code.claude.com/docs/en/setup),
[Codex](https://github.com/openai/codex),
[Gemini CLI](https://github.com/google-gemini/gemini-cli),
Grok Build or [OpenCode](https://opencode.ai/docs/).

Data lives in `%APPDATA%\com.agentdeck.app`, shared with the previous Tauri AgentDeck (0.1.x): close that one
first, or set `AGENTDECK_DATA_DIR` to another folder to keep them apart.

## Shortcuts

| Keys | Action |
| --- | --- |
| Ctrl+K | Command palette |
| Ctrl+Space | Hold to talk |
| Ctrl+1 ... Ctrl+9 | Focus pane 1 to 9 |
| Ctrl+Shift+A | Activity panel |
| Ctrl+Shift+B | Broadcast a prompt |
| Ctrl+Shift+F | Search terminal history |
| Ctrl+Shift+J | Jump to the agent waiting for you |
| Ctrl+Shift+W | Wall view |
| Ctrl+, | Settings |
| Ctrl+Shift+= / Ctrl+Shift+- / Ctrl+Shift+0 | Zoom the interface |
| F2 | Rename the session |

## Build from source

Requirements: Rust (the version is pinned in `rust-toolchain.toml`, MSVC on Windows), and LLVM (clang-cl) + CMake + Ninja for whisper.cpp: `.cargo/config.toml` builds it with clang-cl,
which transcribes about 8x faster than an MSVC build.

```sh
cargo build --release -p agentdeck   # binary in target/release/agentdeck.exe
cargo test --workspace
```

- `crates/core`: backend (PTY, hooks server, permissions, git, history, MCP, voice, usage, persistence)
- `crates/app`: the GPUI interface

See `PORT.md` for the mapping from the original Tauri + React app.

## Privacy

Everything runs on your machine. The hook server only listens on 127.0.0.1 with a per-run token, voice
transcription is local by default, and terminal history stays in your app data folder (you can turn it off).
