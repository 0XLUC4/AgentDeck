// Small form primitives shared by the Settings sections.
import type { ReactNode } from "react";

export const field =
  "h-8 min-w-0 rounded-md border border-line bg-base px-2.5 text-fg placeholder:text-subtle outline-none transition-colors duration-150 hover:border-line-strong focus:border-accent disabled:opacity-50";

export const button =
  "inline-flex h-8 shrink-0 items-center gap-1.5 rounded-md border border-line px-3 text-muted transition-colors duration-150 hover:bg-hover hover:text-fg disabled:pointer-events-none disabled:opacity-40";

export function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col">
      <h3 className="mb-1 text-[15px] font-medium text-fg">{title}</h3>
      <div className="flex flex-col divide-y divide-line">{children}</div>
    </section>
  );
}

/** One setting: label (+ optional hint) on the left, control on the right. */
export function Row({
  label,
  hint,
  htmlFor,
  children,
}: {
  label: string;
  hint?: string;
  htmlFor?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-6 py-3">
      <div className="flex min-w-0 flex-col gap-0.5">
        <label htmlFor={htmlFor} className="text-fg">
          {label}
        </label>
        {hint && <span className="text-xs text-subtle">{hint}</span>}
      </div>
      <div className="flex shrink-0 items-center gap-2">{children}</div>
    </div>
  );
}

export function Toggle({
  id,
  checked,
  onChange,
  label,
}: {
  id?: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  label?: string;
}) {
  return (
    <button
      id={id}
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={() => onChange(!checked)}
      className={`relative inline-flex h-5 w-9 shrink-0 items-center rounded-full border transition-colors duration-150 ${
        checked ? "border-accent bg-accent" : "border-line-strong bg-base hover:border-subtle"
      }`}
    >
      <span
        className={`size-3.5 rounded-full transition-transform duration-150 ease-out-quart ${
          checked ? "translate-x-[18px] bg-on-accent" : "translate-x-[2px] bg-subtle"
        }`}
      />
    </button>
  );
}

export function ErrorText({ children }: { children: ReactNode }) {
  return (
    <p role="alert" className="rounded-md bg-bad/10 px-3 py-2 text-xs text-bad">
      {children}
    </p>
  );
}

export const errorMessage = (err: unknown) => (err instanceof Error ? err.message : String(err));
