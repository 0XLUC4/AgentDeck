import { X } from "lucide-react";
import { type ReactNode, useEffect, useId, useRef } from "react";
import { createPortal } from "react-dom";
import { IconButton } from "../IconButton/IconButton";

/**
 * Modal dialog rendered in <body> (so it never inherits the title bar's drag region).
 * Esc and the close button call `onClose`.
 */
export function Modal({
  title,
  onClose,
  actions,
  className = "w-[min(44rem,94vw)]",
  children,
}: {
  title: string;
  onClose: () => void;
  /** Extra header buttons, left of "Fermer". */
  actions?: ReactNode;
  className?: string;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => ref.current?.showModal(), []);

  return createPortal(
    <dialog
      ref={ref}
      onClose={onClose}
      aria-labelledby={titleId}
      className={`m-auto max-h-[86vh] rounded-xl border border-line bg-panel p-0 text-fg backdrop:bg-gutter/70 ${className}`}
    >
      <div className="flex max-h-[86vh] flex-col">
        <div className="flex shrink-0 items-center gap-2 border-b border-line px-5 py-3">
          <h2 id={titleId} className="flex-1 truncate text-[15px] font-medium">
            {title}
          </h2>
          {actions}
          <IconButton icon={X} label="Fermer" onClick={onClose} />
        </div>
        <div className="scroll-thin min-h-0 flex-1 overflow-y-auto px-5 py-4">{children}</div>
      </div>
    </dialog>,
    document.body,
  );
}

/** Segmented control (one pressed button) used by the dialogs. */
export function Segmented<T extends string>({
  label,
  options,
  value,
  onChange,
}: {
  label: string;
  options: { id: T; label: string }[];
  value: T;
  onChange: (id: T) => void;
}) {
  return (
    <fieldset className="inline-flex gap-1 rounded-lg bg-base p-1">
      <legend className="sr-only">{label}</legend>
      {options.map((o) => (
        <button
          key={o.id}
          type="button"
          aria-pressed={value === o.id}
          onClick={() => onChange(o.id)}
          className={`h-7 rounded-md px-3 text-xs transition-colors duration-150 ${
            value === o.id ? "bg-raised font-medium text-fg" : "text-muted hover:text-fg"
          }`}
        >
          {o.label}
        </button>
      ))}
    </fieldset>
  );
}
