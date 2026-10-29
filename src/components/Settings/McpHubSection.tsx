import { Download, Play, Plus, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { type HubServer, hubClaudeServers, hubList, hubSave, hubTest } from "../../lib/ipc";
import { splitArgs } from "../../lib/splitArgs";
import { button, ErrorText, errorMessage, field, Section } from "./controls";
import type { Save } from "./SettingsDialog";

const EMPTY: HubServer = { name: "", command: "", args: [], env: {}, url: "", headers: {} };

/** What a row shows: the URL, or the command line. */
const target = (s: HubServer) => s.url || [s.command, ...s.args].join(" ");

/** Shared MCP servers: one process run by AgentDeck, used by every agent pane. */
export function McpHubSection({ save }: { save: Save }) {
  const [servers, setServers] = useState<HubServer[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [tests, setTests] = useState<Record<string, string>>({});
  const [name, setName] = useState("");
  const [line, setLine] = useState("");

  useEffect(() => {
    hubList().then(setServers, (err) => setError(errorMessage(err)));
  }, []);

  function persist(next: HubServer[]) {
    const task = hubSave(next);
    task.then(
      () => {
        setError(null);
        setServers(next);
      },
      (err) => setError(errorMessage(err)),
    );
    save(task);
  }

  function add() {
    const text = line.trim();
    const isUrl = /^https?:\/\//i.test(text);
    const [command = "", ...args] = isUrl ? [] : splitArgs(text);
    persist([...servers, { ...EMPTY, name: name.trim(), url: isUrl ? text : "", command, args }]);
    setName("");
    setLine("");
  }

  async function importClaude() {
    try {
      const found = await hubClaudeServers();
      const fresh = found.filter((f) => !servers.some((s) => s.name === f.name));
      if (fresh.length === 0) return setError("No new server in ~/.claude.json.");
      persist([...servers, ...fresh]);
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  async function test(serverName: string) {
    setTests((t) => ({ ...t, [serverName]: "Starting…" }));
    const result = await hubTest(serverName).then(
      (count) => `Works: ${count} tools`,
      (err) => `Failed: ${errorMessage(err)}`,
    );
    setTests((t) => ({ ...t, [serverName]: result }));
  }

  return (
    <Section title="MCP hub">
      <p className="py-3 text-xs text-subtle">
        AgentDeck runs one copy of each server below and every agent (Claude, Codex, Grok) goes through it, under the
        same name. Use it for servers that accept a single client, like Roblox Studio: agents no longer disconnect each
        other, and tool calls run one at a time. Applies to agents started after a change.
      </p>
      {error && <ErrorText>{error}</ErrorText>}
      {servers.map((s) => (
        <div key={s.name} className="flex items-center gap-3 py-3">
          <div className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="truncate text-fg">{s.name}</span>
            <span className="truncate font-mono text-xs text-subtle" title={target(s)}>
              {target(s)}
            </span>
            {tests[s.name] && (
              <span
                className={`whitespace-pre-wrap text-xs ${tests[s.name].startsWith("Failed") ? "text-bad" : "text-muted"}`}
              >
                {tests[s.name]}
              </span>
            )}
          </div>
          <button type="button" className={button} onClick={() => test(s.name)}>
            <Play size={13} strokeWidth={1.5} aria-hidden="true" />
            Test
          </button>
          <button
            type="button"
            className={button}
            aria-label={`Remove ${s.name}`}
            onClick={() => persist(servers.filter((o) => o.name !== s.name))}
          >
            <Trash2 size={13} strokeWidth={1.5} aria-hidden="true" />
          </button>
        </div>
      ))}
      <form
        className="flex items-center gap-2 py-3"
        onSubmit={(e) => {
          e.preventDefault();
          add();
        }}
      >
        <input
          aria-label="Server name"
          value={name}
          placeholder="Name (e.g. roblox-studio-plus)"
          onChange={(e) => setName(e.target.value)}
          className={`${field} w-48`}
        />
        <input
          aria-label="Command or URL"
          value={line}
          placeholder="node D:/server/index.js  or  https://…/mcp"
          onChange={(e) => setLine(e.target.value)}
          className={`${field} flex-1 font-mono text-xs`}
        />
        <button type="submit" className={button} disabled={!name.trim() || !line.trim()}>
          <Plus size={13} strokeWidth={1.5} aria-hidden="true" />
          Add
        </button>
      </form>
      <div className="py-3">
        <button type="button" className={button} onClick={importClaude}>
          <Download size={13} strokeWidth={1.5} aria-hidden="true" />
          Import from Claude Code
        </button>
      </div>
    </Section>
  );
}
