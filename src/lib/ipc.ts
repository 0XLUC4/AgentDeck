// Typed wrappers around Tauri commands and events. Single source of IPC names.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AgentInfo, AgentStatusEvent, SpawnOptions, VoiceState } from "./types";

export const spawnPane = (opts: SpawnOptions) => invoke<void>("spawn_pane", { ...opts });
export const writeToPane = (paneId: string, data: string) => invoke<void>("write_to_pane", { paneId, data });
export const resizePane = (paneId: string, cols: number, rows: number) =>
  invoke<void>("resize_pane", { paneId, cols, rows });
export const killPane = (paneId: string) => invoke<void>("kill_pane", { paneId });
export const setActivePane = (paneId: string | null) => invoke<void>("set_active_pane", { paneId });
export const detectAgents = () => invoke<AgentInfo[]>("detect_agents");

export const voiceStart = () => invoke<void>("voice_start");
export const voiceStop = () => invoke<void>("voice_stop");
export const voiceSend = (text: string, submit: boolean) => invoke<void>("voice_send", { text, submit });

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
