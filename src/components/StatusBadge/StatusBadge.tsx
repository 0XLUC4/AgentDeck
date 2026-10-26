import type { PaneStatus } from "../../lib/types";

const STATUS: Record<PaneStatus, { label: string; dot: string }> = {
  done: { label: "Terminé", dot: "bg-green-500" },
  working: { label: "En cours", dot: "bg-blue-500 animate-pulse" },
  waiting: { label: "En attente", dot: "bg-orange-500" },
  error: { label: "Erreur", dot: "bg-red-500" },
  idle: { label: "Inactif", dot: "bg-zinc-400" },
};

export function StatusBadge({ status }: { status: PaneStatus }) {
  const { label, dot } = STATUS[status];
  return (
    <span
      className="inline-flex shrink-0 items-center gap-1.5 rounded-full bg-zinc-800 px-2 py-0.5 text-xs"
      title={`Statut : ${label}`}
    >
      <span className={`size-2 rounded-full ${dot}`} aria-hidden="true" />
      {label}
    </span>
  );
}
