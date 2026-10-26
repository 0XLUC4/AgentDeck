// Typed wrappers around Tauri commands and events. Single source of IPC names.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AgentInfo,
  AgentKind,
  AgentStatusEvent,
  FolderInfo,
  GitCheckpoint,
  GitCommitInfo,
  GitComparison,
  GitStatus,
  HistoryHit,
  HistoryWindow,
  McpActivity,
  McpRequest,
  ModelStatus,
  OpenProject,
  PaneUsage,
  PermissionDecision,
  PermissionRequest,
  SpawnOptions,
  UsageRange,
  UsageSummary,
  VoiceConfig,
  VoiceDownload,
  VoiceState,
  WhisperModel,
} from "./types";

export const spawnPane = (opts: SpawnOptions) => invoke<void>("spawn_pane", { ...opts });
export const writeToPane = (paneId: string, data: string) => invoke<void>("write_to_pane", { paneId, data });
export const resizePane = (paneId: string, cols: number, rows: number) =>
  invoke<void>("resize_pane", { paneId, cols, rows });
export const killPane = (paneId: string) => invoke<void>("kill_pane", { paneId });
export const setActivePane = (paneId: string | null) => invoke<void>("set_active_pane", { paneId });
export const detectAgents = () => invoke<AgentInfo[]>("detect_agents");
export const uninstallHooks = (cwd: string) => invoke<void>("uninstall_hooks", { cwd });
export const listOpenProjects = () => invoke<OpenProject[]>("list_open_projects");
export const inspectFolder = (path: string) => invoke<FolderInfo>("inspect_folder", { path });
export const setTrayStatus = (waiting: number) => invoke<void>("set_tray_status", { waiting });
export const setCloseToTray = (enabled: boolean) => invoke<void>("set_close_to_tray", { enabled });

export const voiceStart = () => invoke<void>("voice_start");
export const voiceStop = () => invoke<void>("voice_stop");
export const voiceSend = (text: string, submit: boolean) => invoke<void>("voice_send", { text, submit });
export const getVoiceConfig = () => invoke<VoiceConfig>("voice_get_config");
export const setVoiceConfig = (config: Omit<VoiceConfig, "hasApiKey">) =>
  invoke<VoiceConfig>("voice_set_config", { config });
export const setVoiceApiKey = (key: string | null) => invoke<void>("voice_set_api_key", { key });
export const listWhisperModels = () => invoke<ModelStatus[]>("voice_list_models");
export const downloadWhisperModel = (model: WhisperModel) => invoke<void>("voice_download_model", { model });
// event "voice-download" payload { received: number; total: number | null }

export const onPaneOutput = (paneId: string, cb: (data: string) => void): Promise<UnlistenFn> =>
  listen<string>(`pane-output:${paneId}`, (e) => cb(e.payload));
export const onPaneExit = (paneId: string, cb: () => void): Promise<UnlistenFn> =>
  listen(`pane-exit:${paneId}`, () => cb());
export const onAgentStatus = (cb: (e: AgentStatusEvent) => void): Promise<UnlistenFn> =>
  listen<AgentStatusEvent>("agent-status", (e) => cb(e.payload));

export const onVoiceLevel = (cb: (level: number) => void) => listen<number>("voice-level", (e) => cb(e.payload));
export const onVoiceState = (cb: (state: VoiceState) => void) =>
  listen<VoiceState>("voice-state", (e) => cb(e.payload));
export const onVoiceTranscript = (cb: (text: string) => void) =>
  listen<string>("voice-transcript", (e) => cb(e.payload));
export const onVoiceError = (cb: (message: string) => void) => listen<string>("voice-error", (e) => cb(e.payload));
export const onVoiceDownload = (cb: (progress: VoiceDownload) => void) =>
  listen<VoiceDownload>("voice-download", (e) => cb(e.payload));

// Git panel (src-tauri/src/git). `cwd` is the pane's directory as returned by gitPaneDir.
export const gitPaneDir = (cwd: string, paneId: string, worktree: boolean) =>
  invoke<string>("git_pane_dir", { cwd, paneId, worktree });
export const gitStatus = (cwd: string) => invoke<GitStatus>("git_status", { cwd });
export const gitDiff = (cwd: string, path: string) => invoke<string>("git_diff", { cwd, path });
export const gitDiffTrees = (cwd: string, from: string, to: string, path: string) =>
  invoke<string>("git_diff_trees", { cwd, from, to, path });
export const gitCommit = (cwd: string, message: string) => invoke<string>("git_commit", { cwd, message });
export const gitMergeWorktree = (cwd: string) => invoke<string>("git_merge_worktree", { cwd });
export const gitRemoveWorktree = (cwd: string) => invoke<void>("git_remove_worktree", { cwd });
export const gitCheckpoint = (cwd: string, paneId: string) => invoke<string | null>("git_checkpoint", { cwd, paneId });
export const gitListCheckpoints = (cwd: string, paneId: string) =>
  invoke<GitCheckpoint[]>("git_list_checkpoints", { cwd, paneId });
/** Returns the sha of the safety checkpoint taken just before restoring. */
export const gitRestoreCheckpoint = (cwd: string, paneId: string, sha: string) =>
  invoke<string | null>("git_restore_checkpoint", { cwd, paneId, sha });
export const gitCompare = (cwd: string, otherCwd: string) => invoke<GitComparison>("git_compare", { cwd, otherCwd });

export const answerPermission = (requestId: string, decision: PermissionDecision, message?: string) =>
  invoke<void>("answer_permission", { requestId, decision, message: message ?? null });
export const onAgentPermission = (cb: (req: PermissionRequest) => void): Promise<UnlistenFn> =>
  listen<PermissionRequest>("agent-permission", (e) => cb(e.payload));
export const onAgentPermissionResolved = (cb: (requestId: string) => void): Promise<UnlistenFn> =>
  listen<{ requestId: string }>("agent-permission-resolved", (e) => cb(e.payload.requestId));

// Orchestrator MCP bridge (src-tauri/src/mcp). `mcpRegister` returns the Claude args to prepend and the
// per-spawn secret to pass as env to spawn_pane only (never store it).
export const mcpRegister = (paneId: string) => invoke<{ args: string[]; secret: string }>("mcp_register", { paneId });
export const mcpRespond = (requestId: string, result: unknown, error: string | null) =>
  invoke<void>("mcp_respond", { requestId, result, error });
export const mcpSetPaused = (paused: boolean) => invoke<void>("mcp_set_paused", { paused });
export const onMcpRequest = (cb: (req: McpRequest) => void): Promise<UnlistenFn> =>
  listen<McpRequest>("mcp-request", (e) => cb(e.payload));
export const onMcpActivity = (cb: (e: McpActivity) => void): Promise<UnlistenFn> =>
  listen<McpActivity>("mcp-activity", (e) => cb(e.payload));

export const usageSummary = (range: UsageRange) => invoke<UsageSummary>("usage_summary", { range });
export const paneUsage = (paneId: string, cwd: string, startedAt: number, agent: AgentKind) =>
  invoke<PaneUsage>("pane_usage", { paneId, cwd, startedAt, agent });

export const historySearch = (query: string, limit: number) => invoke<HistoryHit[]>("history_search", { query, limit });
export const historyRead = (runId: string, aroundLine: number) =>
  invoke<HistoryWindow>("history_read", { runId, aroundLine });
export const historyGetEnabled = () => invoke<boolean>("history_get_enabled");
export const historySetEnabled = (enabled: boolean) => invoke<void>("history_set_enabled", { enabled });
export const historyClear = () => invoke<void>("history_clear");

/** `entry` is a JournalEntry (src/components/Journal/journal.ts), appended to today's file. */
export const journalAppend = (entry: unknown) => invoke<void>("journal_append", { entry });
export const journalRead = (date: string) => invoke<unknown[]>("journal_read", { date });
export const journalGitCommits = (cwd: string) => invoke<GitCommitInfo[]>("journal_git_commits", { cwd });
/** Opens a native save dialog, then writes. Resolves to the saved path, or null if cancelled. */
export const writeTextFile = (defaultName: string, contents: string) =>
  invoke<string | null>("write_text_file", { defaultName, contents });
