// Settings > Appearance: light / dark / system theme, the app's accent color and its background tint.
import { Check } from "lucide-react";
import { useEffect, useRef } from "react";
import { ACCENT_PRESETS, BACKGROUND_PRESETS } from "../../lib/theme";
import { type Appearance, type ThemeMode, updateSettings, useSettings } from "../../store/settings";
import { Segmented } from "../Usage/Modal";
import { Row, Section } from "./controls";
import type { Save } from "./SettingsDialog";

const MODES: { id: ThemeMode; label: string }[] = [
  { id: "dark", label: "Dark" },
  { id: "light", label: "Light" },
  { id: "system", label: "System" },
];

/** The color picker fires on every drag step: preview live, write to disk once it settles. */
const PICKER_SAVE_MS = 300;

interface SwatchesProps {
  label: string;
  presets: { label: string; value: string }[];
  value: string;
  /** What the "" (neutral) swatch looks like. */
  neutral: string;
  /** Check mark color on a selected colored swatch. */
  checkClass: string;
  onSet: (value: string) => void;
  onPick: (value: string) => void;
}

function Swatches({ label, presets, value, neutral, checkClass, onSet, onPick }: SwatchesProps) {
  const custom = value !== "" && !presets.some((p) => p.value === value);
  return (
    <fieldset className="flex items-center gap-1.5">
      <legend className="sr-only">{label}</legend>
      {presets.map((p) => {
        const selected = value === p.value;
        return (
          <button
            key={p.label}
            type="button"
            title={p.label}
            aria-label={p.label}
            aria-pressed={selected}
            onClick={() => onSet(p.value)}
            className="relative grid size-6 place-items-center rounded-full ring-1 ring-line-strong transition-transform duration-150 hover:scale-110 aria-pressed:ring-2 aria-pressed:ring-fg"
            style={{ background: p.value || neutral }}
          >
            {selected && (
              <Check size={12} strokeWidth={2.5} aria-hidden="true" className={p.value ? checkClass : "text-base"} />
            )}
          </button>
        );
      })}
      <label
        title="Custom color"
        className={`relative size-6 cursor-pointer overflow-hidden rounded-full ring-1 ring-line-strong transition-transform duration-150 hover:scale-110 ${
          custom ? "ring-2 ring-fg" : ""
        }`}
        style={{
          background: custom ? value : "conic-gradient(#f87171, #fbbf24, #4ade80, #60a5fa, #a78bfa, #f472b6, #f87171)",
        }}
      >
        <span className="sr-only">Custom {label.toLowerCase()}</span>
        <input
          type="color"
          value={value || "#4f8ff7"}
          onChange={(e) => onPick(e.target.value.toLowerCase())}
          className="absolute inset-0 cursor-pointer opacity-0"
        />
      </label>
    </fieldset>
  );
}

export function AppearanceSection({ save }: { save: Save }) {
  const appearance = useSettings((s) => s.appearance);
  const pickerTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(pickerTimer.current), []);

  const set = (patch: Partial<Appearance>) => save(updateSettings({ appearance: { ...appearance, ...patch } }));
  const pick = (key: "accent" | "background") => (color: string) => {
    useSettings.setState({ appearance: { ...appearance, [key]: color } });
    clearTimeout(pickerTimer.current);
    pickerTimer.current = setTimeout(() => save(updateSettings({})), PICKER_SAVE_MS);
  };

  return (
    <Section title="Appearance">
      <Row label="Theme" hint="System follows Windows' light/dark setting. Terminals stay dark.">
        <Segmented label="Theme" options={MODES} value={appearance.theme} onChange={(theme) => set({ theme })} />
      </Row>
      <Row label="Accent color" hint="Buttons, focus rings, toggles and highlights">
        <Swatches
          label="Accent color"
          presets={ACCENT_PRESETS}
          value={appearance.accent}
          // Neutral shows the theme's own accent (light grey in dark mode, near-black in light mode).
          neutral="var(--color-fg)"
          checkClass="text-on-accent"
          onSet={(accent) => set({ accent })}
          onPick={pick("accent")}
        />
      </Row>
      <Row label="Background" hint="Tints windows and panels in the theme's own light or dark shade">
        <Swatches
          label="Background"
          presets={BACKGROUND_PRESETS}
          value={appearance.background}
          neutral="var(--color-raised)"
          checkClass="text-white"
          onSet={(background) => set({ background })}
          onPick={pick("background")}
        />
      </Row>
    </Section>
  );
}
