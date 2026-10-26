import type { LucideIcon } from "lucide-react";
import type { ButtonHTMLAttributes } from "react";

interface Props extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "children"> {
  icon: LucideIcon;
  /** Accessible name; also the tooltip unless `hint` is given. */
  label: string;
  /** Longer tooltip (shortcut, consequence). */
  hint?: string;
  tone?: "default" | "danger";
  size?: "sm" | "md";
}

const TONE = {
  default: "text-subtle hover:bg-hover hover:text-fg",
  danger: "text-subtle hover:bg-bad/15 hover:text-bad",
};

/** Square icon-only button: every action in the app goes through this, so they all look and behave the same. */
export function IconButton({ icon: Icon, label, hint, tone = "default", size = "md", className = "", ...rest }: Props) {
  const box = size === "sm" ? "size-6" : "size-7";
  return (
    <button
      type="button"
      aria-label={label}
      title={hint ?? label}
      className={`inline-flex ${box} shrink-0 items-center justify-center rounded-md transition-colors duration-150 disabled:pointer-events-none disabled:opacity-35 aria-pressed:bg-raised aria-pressed:text-fg ${TONE[tone]} ${className}`}
      {...rest}
    >
      <Icon size={size === "sm" ? 14 : 16} strokeWidth={1.5} aria-hidden="true" />
    </button>
  );
}
