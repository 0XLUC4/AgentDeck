# AgentDeck

AgentDeck, rewritten in pure Rust with [GPUI](https://www.gpui.rs) and gpui-component.
Run many AI coding agents (Claude Code, Codex, Gemini CLI, Grok Build, OpenCode, shell) side by side,
each in its own GPU-rendered terminal, with splits and tabs, live status, permission prompts, Git,
history, voice, an MCP hub and more.

Windows first. Early version: expect rough edges.

## Run

Download `AgentDeck_0.2.0_x64-portable.zip` from the releases, unzip and start `agentdeck.exe`.
It shares its data folder (`%APPDATA%\com.agentdeck.app`) with the previous Tauri AgentDeck (0.1.x), so close that one first,
or set `AGENTDECK_DATA_DIR` to another folder to keep them apart.

## Build

```sh
cargo build --release -p agentdeck
```

`rust-toolchain.toml` pins the Rust version GPUI needs. The binary lands in `target/release/agentdeck.exe`.

## Layout

- `crates/core`: backend (PTY, hooks server, permissions, git, history, MCP, voice, usage, persistence).
- `crates/app`: the GPUI interface.

See `PORT.md` for the mapping from the original Tauri + React app.
