import type { PaneStatus } from "../../lib/types";

export const STATUS: Record<PaneStatus, { label: string; color: string }> = {
  done: { label: "Terminé", color: "var(--color-ok)" },
  working: { label: "En cours", color: "var(--color-busy)" },
  waiting: { label: "En attente de votre réponse", color: "var(--color-wait)" },
  error: { label: "Erreur", color: "var(--color-bad)" },
  idle: { label: "Inactif", color: "var(--color-subtle)" },
};

export function StatusDot({ status }: { status: PaneStatus }) {
  return (
    <span
      className={`size-2 shrink-0 rounded-full ${status === "waiting" ? "status-dot-pulse" : ""}`}
      style={{ background: STATUS[status].color }}
      aria-hidden="true"
    />
  );
}

/** Status dot; the label is the tooltip and is read by screen readers. */
export function StatusBadge({ status }: { status: PaneStatus }) {
  const { label } = STATUS[status];
  return (
    <span className="inline-flex size-4 shrink-0 items-center justify-center" title={`Statut : ${label}`}>
      <StatusDot status={status} />
      <span className="sr-only">{label}</span>
    </span>
  );
}
