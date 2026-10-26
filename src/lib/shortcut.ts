// Converts keyboard events to Tauri global-shortcut strings ("ctrl+shift+f9") and back to a readable label.

export interface KeyCombo {
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  /** KeyboardEvent.code (layout independent): "KeyA", "Digit1", "F9", "Space"... */
  code: string;
}

const MODIFIER_CODES = /^(Control|Shift|Alt|Meta|OS)(Left|Right)?$/;

/** Tauri shortcut string for a key press, or null while only modifiers are held. */
export function toTauriShortcut(e: KeyCombo): string | null {
  if (!e.code || MODIFIER_CODES.test(e.code)) return null;
  const key = e.code.replace(/^(Key|Digit)/, "").toLowerCase();
  const mods = [e.ctrlKey && "ctrl", e.altKey && "alt", e.shiftKey && "shift", e.metaKey && "super"];
  return [...mods.filter(Boolean), key].join("+");
}

const LABELS: Record<string, string> = {
  ctrl: "Ctrl",
  control: "Ctrl",
  commandorcontrol: "Ctrl",
  alt: "Alt",
  shift: "Shift",
  super: "Win",
  space: "Space",
  enter: "Enter",
  backspace: "Backspace",
  arrowup: "↑",
  arrowdown: "↓",
  arrowleft: "←",
  arrowright: "→",
};

/** "ctrl+space" -> "Ctrl + Space". */
export const formatShortcut = (shortcut: string) =>
  shortcut
    .split("+")
    .filter(Boolean)
    .map((part) => LABELS[part.toLowerCase()] ?? part.charAt(0).toUpperCase() + part.slice(1))
    .join(" + ");
