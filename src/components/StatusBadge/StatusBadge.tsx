import type { PaneStatus } from "../../lib/types";

export const STATUS: Record<PaneStatus, { label: string; color: string }> = {
  done: { label: "Done", color: "var(--color-ok)" },
  working: { label: "Working", color: "var(--color-busy)" },
  waiting: { label: "Waiting for your reply", color: "var(--color-wait)" },
  error: { label: "Error", color: "var(--color-bad)" },
  idle: { label: "Idle", color: "var(--color-subtle)" },
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
    <span className="inline-flex size-4 shrink-0 items-center justify-center" title={`Status: ${label}`}>
      <StatusDot status={status} />
      <span className="sr-only">{label}</span>
    </span>
  );
}
