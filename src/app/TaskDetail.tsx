import { ArrowLeft, Check, Copy, Dot, FilePen, type LucideIcon, Search, SquareTerminal, Terminal } from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { AgentLogo } from "../components/AgentLogo/AgentLogo";
import { AGENTS } from "../lib/agents";
import { formatElapsed, formatSince, useNow } from "../lib/clock";
import { type ActivityItem, deck } from "../store/deck";
import { type Task, useTasks } from "../store/tasks";

const LONG_PROMPT = 400;

function actionIcon(text: string): LucideIcon {
  if (text.startsWith("Bash")) return Terminal;
  if (/^(Edit|MultiEdit|Write|NotebookEdit)\b/.test(text)) return FilePen;
  if (/^(Read|Grep|Glob)\b/.test(text)) return Search;
  return Dot;
}

const BUTTON =
  "inline-flex h-7 items-center gap-1.5 rounded-md border border-line px-2 text-xs text-muted transition-colors duration-150 hover:bg-hover hover:text-fg";

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-1.5 border-b border-line px-3 py-2.5">
      <h4 className="text-xs text-subtle">{title}</h4>
      {children}
    </section>
  );
}

function Prompt({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  const long = text.length > LONG_PROMPT;
  return (
    <>
      <p className={`whitespace-pre-wrap break-words font-mono text-xs text-fg ${long && !open ? "line-clamp-6" : ""}`}>
        {text}
      </p>
      {long && (
        <button type="button" className="w-fit text-xs text-subtle hover:text-fg" onClick={() => setOpen(!open)}>
          {open ? "Réduire" : "Tout afficher"}
        </button>
      )}
    </>
  );
}

/** Prompt, actions and result of one task. Plain React text only: agent output is never parsed as HTML. */
function TaskBody({ task }: { task: Task }) {
  const now = useNow();
  return (
    <>
      <Section title="Demande">
        {task.prompt ? <Prompt text={task.prompt} /> : <p className="text-xs text-subtle">Demande non reçue.</p>}
      </Section>
      <Section title={`Actions (${task.actions.length})`}>
        {task.actions.length === 0 ? (
          <p className="text-xs text-subtle">Aucune action outil.</p>
        ) : (
          <ol className="flex flex-col gap-1">
            {task.actions.map((action) => {
              const Icon = actionIcon(action.text);
              return (
                <li key={`${action.at}-${action.text}`} className="flex items-start gap-1.5 text-xs">
                  <Icon size={13} strokeWidth={1.5} className="mt-px shrink-0 text-subtle" aria-hidden="true" />
                  <span className="min-w-0 flex-1 truncate text-muted" title={action.text}>
                    {action.text}
                  </span>
                  <span className="shrink-0 tabular-nums text-subtle">{formatSince(now - action.at)}</span>
                </li>
              );
            })}
          </ol>
        )}
      </Section>
      <Section title={task.status === "error" ? "Erreur" : "Résultat"}>
        {task.summary ? (
          <p className="whitespace-pre-wrap break-words text-xs leading-relaxed text-fg">{task.summary}</p>
        ) : (
          <p className="text-xs text-subtle">{task.endedAt ? "Aucun résumé reçu." : "Tâche en cours…"}</p>
        )}
      </Section>
    </>
  );
}

const firstLine = (task: Task) => task.prompt?.split("\n", 1)[0] || "Tâche sans demande";

/** Slide-over inside the activity panel: what a finished agent did. */
export function TaskDetail({ item, onJump, onClose }: { item: ActivityItem; onJump: () => void; onClose: () => void }) {
  const { pane } = item;
  const log = useTasks((s) => s[pane.id]);
  const now = useNow();
  const back = useRef<HTMLButtonElement>(null);
  const [copied, setCopied] = useState(false);
  const [task, ...previous] = log ? (log.current ? [log.current, ...log.history] : log.history) : [];
  const name = pane.title || AGENTS[pane.agent].name;

  useEffect(() => back.current?.focus(), []);

  const copy = () => {
    if (!task?.summary) return;
    navigator.clipboard.writeText(task.summary).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      },
      () => deck.toast("Impossible de copier le résultat"),
    );
  };

  return (
    <div
      className="absolute inset-0 z-10 flex flex-col bg-panel"
      role="dialog"
      aria-label={`Détail de la tâche : ${name}`}
      onKeyDown={(e) => e.key === "Escape" && onClose()}
    >
      <div className="flex h-10 shrink-0 items-center gap-2 border-b border-line pr-2 pl-1.5">
        <button
          ref={back}
          type="button"
          aria-label="Retour à l'activité"
          title="Retour (Échap)"
          className="inline-flex size-7 items-center justify-center rounded-md text-subtle hover:bg-hover hover:text-fg"
          onClick={onClose}
        >
          <ArrowLeft size={16} strokeWidth={1.5} aria-hidden="true" />
        </button>
        <AgentLogo kind={pane.agent} size={14} />
        <span className="min-w-0 flex-1 truncate font-medium text-fg">{name}</span>
        {task && (
          <span className="shrink-0 text-xs tabular-nums text-subtle" title="Durée">
            {formatElapsed((task.endedAt ?? now) - task.startedAt)}
          </span>
        )}
      </div>
      <div className="scroll-thin min-h-0 flex-1 overflow-y-auto">
        {task ? (
          <TaskBody task={task} />
        ) : (
          <p className="px-3 py-4 text-xs text-subtle">Aucun détail disponible pour cet agent (pas de hooks).</p>
        )}
        <div className="flex flex-wrap gap-1.5 border-b border-line px-3 py-2.5">
          <button type="button" className={BUTTON} onClick={onJump}>
            <SquareTerminal size={13} strokeWidth={1.5} aria-hidden="true" />
            Aller au terminal
          </button>
          {pane.status === "done" && (
            <button
              type="button"
              className={BUTTON}
              onClick={() => {
                deck.acknowledge(pane.id);
                onClose();
              }}
            >
              <Check size={13} strokeWidth={1.5} aria-hidden="true" />
              Marquer comme vu
            </button>
          )}
          {task?.summary && (
            <button type="button" className={BUTTON} onClick={copy}>
              <Copy size={13} strokeWidth={1.5} aria-hidden="true" />
              {copied ? "Copié" : "Copier le résultat"}
            </button>
          )}
        </div>
        {previous.length > 0 && (
          <Section title={`Tâches précédentes (${previous.length})`}>
            <ul className="flex flex-col">
              {previous.map((prev) => (
                <li key={prev.startedAt}>
                  <details>
                    <summary className="flex cursor-pointer items-center gap-2 rounded-md px-1 py-1 text-xs text-muted hover:bg-hover hover:text-fg">
                      <span className="min-w-0 flex-1 truncate">{firstLine(prev)}</span>
                      <span className="shrink-0 tabular-nums text-subtle">
                        {formatSince(now - (prev.endedAt ?? prev.startedAt))}
                      </span>
                    </summary>
                    <div className="-mx-3 border-t border-line">
                      <TaskBody task={prev} />
                    </div>
                  </details>
                </li>
              ))}
            </ul>
          </Section>
        )}
      </div>
    </div>
  );
}
