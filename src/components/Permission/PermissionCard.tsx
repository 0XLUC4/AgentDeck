import { FilePen, FileText, Globe, type LucideIcon, Plug, ShieldQuestion, SquareTerminal } from "lucide-react";
import { useState } from "react";
import type { PermissionRequest } from "../../lib/types";
import { answer } from "./permissions";

const TOOL_ICON: Record<string, LucideIcon> = {
  Bash: SquareTerminal,
  PowerShell: SquareTerminal,
  Edit: FilePen,
  MultiEdit: FilePen,
  Write: FilePen,
  NotebookEdit: FilePen,
  Read: FileText,
  WebFetch: Globe,
  WebSearch: Globe,
};

const toolIcon = (tool: string) => TOOL_ICON[tool] ?? (tool.startsWith("mcp__") ? Plug : ShieldQuestion);
const isShell = (tool: string) => tool === "Bash" || tool === "PowerShell";

const BUTTON = "h-6 rounded px-2 text-xs font-medium transition-colors duration-150";
const PRIMARY = `${BUTTON} bg-accent text-on-accent hover:bg-accent-hover`;
const SECONDARY = `${BUTTON} border border-line-strong text-fg hover:bg-hover`;
const DANGER = `${BUTTON} text-bad hover:bg-bad/15`;

/** A pending tool approval: exactly what will run, and Autoriser / Toujours autoriser / Refuser. Never auto-allows. */
export function PermissionCard({ request, onJump }: { request: PermissionRequest; onJump?: () => void }) {
  const [refusing, setRefusing] = useState(false);
  const [reason, setReason] = useState("");
  const Icon = toolIcon(request.toolName);
  const detail = (
    <pre className="scroll-thin max-h-40 overflow-auto rounded bg-gutter p-1.5 font-mono text-[11px] whitespace-pre-wrap break-all text-fg">
      {request.detail}
    </pre>
  );
  const deny = () => void answer(request, "deny", reason);

  return (
    <fieldset
      aria-label={`Autorisation demandée : ${request.summary}`}
      className="flex min-w-0 flex-col gap-1.5 rounded-md border border-wait/40 bg-base p-2"
    >
      <p className="flex items-start gap-1.5 text-xs text-fg">
        <Icon size={14} strokeWidth={1.5} className="mt-px shrink-0 text-wait" aria-hidden="true" />
        <span className="min-w-0 flex-1 break-words">{request.summary}</span>
      </p>
      {/* Shell commands are always shown in full: the summary is truncated. */}
      {isShell(request.toolName) ? (
        detail
      ) : (
        <details className="text-xs text-subtle">
          <summary className="cursor-pointer select-none hover:text-fg">Détail</summary>
          <div className="mt-1">{detail}</div>
        </details>
      )}
      {refusing ? (
        <form
          className="flex flex-col gap-1.5"
          onSubmit={(e) => {
            e.preventDefault();
            deny();
          }}
        >
          <input
            // biome-ignore lint/a11y/noAutofocus: opened by an explicit "Refuser" click, the reason is the next step.
            autoFocus
            value={reason}
            onChange={(e) => setReason(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.stopPropagation();
                setRefusing(false);
              }
            }}
            placeholder="Raison (facultatif), transmise à l'agent"
            aria-label="Raison du refus (facultatif)"
            className="h-6 rounded border border-line-strong bg-gutter px-1.5 text-xs text-fg placeholder:text-subtle"
          />
          <span className="flex gap-1.5">
            <button type="submit" className={`${BUTTON} bg-bad/20 text-bad hover:bg-bad/30`}>
              Refuser
            </button>
            <button type="button" className={SECONDARY} onClick={() => setRefusing(false)}>
              Annuler
            </button>
          </span>
        </form>
      ) : (
        <span className="flex flex-wrap gap-1.5">
          <button type="button" className={PRIMARY} onClick={() => void answer(request, "allow")}>
            Autoriser
          </button>
          {request.suggestions.length > 0 && (
            <button
              type="button"
              className={SECONDARY}
              title={`Autorise et ajoute la règle ${request.suggestions.join(", ")} proposée par l'agent`}
              aria-label={`Toujours autoriser ${request.suggestions.join(", ")}`}
              onClick={() => void answer(request, "allow_always")}
            >
              Toujours autoriser
            </button>
          )}
          <button type="button" className={DANGER} onClick={() => setRefusing(true)}>
            Refuser
          </button>
          <button
            type="button"
            className={`${BUTTON} ml-auto text-subtle hover:bg-hover hover:text-fg`}
            title="Rend la main à l'invite de l'agent dans son terminal"
            onClick={() => {
              void answer(request, "ask");
              onJump?.();
            }}
          >
            Terminal
          </button>
        </span>
      )}
    </fieldset>
  );
}
