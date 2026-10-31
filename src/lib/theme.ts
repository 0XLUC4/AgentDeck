// Applies the appearance settings (light/dark/system, accent and background colors) to the document.
// Light mode redefines the color tokens in index.css; the accent overrides --color-accent* inline.
import { type Appearance, HEX_COLOR, settingsReady, useSettings } from "../store/settings";

/** Accent swatches offered in Settings; "" keeps the theme's neutral grey. */
export const ACCENT_PRESETS: { label: string; value: string }[] = [
  { label: "Neutral", value: "" },
  { label: "Orange", value: "#d97757" },
  { label: "Blue", value: "#4f8ff7" },
  { label: "Violet", value: "#9b7fd4" },
  { label: "Green", value: "#3fb67a" },
  { label: "Pink", value: "#e0609a" },
  { label: "Yellow", value: "#e2b33c" },
];

/** Background swatches: the hue tints every surface, lightness stays the theme's. "" = neutral grey. */
export const BACKGROUND_PRESETS: { label: string; value: string }[] = [
  { label: "Neutral", value: "" },
  { label: "Navy", value: "#2b4c8c" },
  { label: "Teal", value: "#1f7a7a" },
  { label: "Forest", value: "#2f7a45" },
  { label: "Plum", value: "#6b3f8c" },
  { label: "Wine", value: "#8c2f4a" },
  { label: "Brown", value: "#7a5230" },
];

/** Surface tokens and their lightness per mode (same values as index.css). */
const SURFACES: Record<"dark" | "light", [string, number][]> = {
  dark: [
    ["gutter", 0.13],
    ["base", 0.165],
    ["panel", 0.195],
    ["raised", 0.235],
    ["hover", 0.27],
    ["line", 0.28],
    ["line-strong", 0.36],
  ],
  light: [
    ["gutter", 0.925],
    ["base", 0.985],
    ["panel", 0.965],
    ["raised", 0.93],
    ["hover", 0.9],
    ["line", 0.87],
    ["line-strong", 0.78],
  ],
};
/** Max tint chroma: enough to read as the color, low enough to keep text contrast. */
const SURFACE_CHROMA = { dark: 0.045, light: 0.03 };

const CACHE_KEY = "appearance";
const lightQuery = () => window.matchMedia("(prefers-color-scheme: light)");

/** sRGB relative luminance (WCAG) of "#rrggbb". */
export function luminance(hex: string): number {
  const [r, g, b] = [1, 3, 5].map((i) => {
    const c = Number.parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** Text color readable on the accent: whichever of near-black / white contrasts more. */
export const onAccent = (hex: string) => (luminance(hex) > 0.18 ? "#141414" : "#ffffff");

export function applyAppearance({ theme, accent, background }: Appearance, root = document.documentElement): void {
  const mode = theme === "system" ? (lightQuery().matches ? "light" : "dark") : theme;
  root.dataset.theme = mode;
  const style = root.style;
  if (HEX_COLOR.test(accent)) {
    style.setProperty("--color-accent", accent);
    style.setProperty(
      "--color-accent-hover",
      `color-mix(in oklch, ${accent} 82%, ${mode === "light" ? "black" : "white"})`,
    );
    style.setProperty("--color-on-accent", onAccent(accent));
  } else {
    for (const name of ["--color-accent", "--color-accent-hover", "--color-on-accent"]) style.removeProperty(name);
  }
  for (const [name, lightness] of SURFACES[mode]) {
    if (HEX_COLOR.test(background)) {
      style.setProperty(`--color-${name}`, `oklch(from ${background} ${lightness} min(c, ${SURFACE_CHROMA[mode]}) h)`);
    } else {
      style.removeProperty(`--color-${name}`);
    }
  }
}

/** Applies the theme now (from a local cache, so a light theme doesn't flash dark), then keeps it in sync. */
export function startAppearance(): void {
  try {
    const cached: unknown = JSON.parse(localStorage.getItem(CACHE_KEY) ?? "null");
    if (cached && typeof cached === "object") applyAppearance({ theme: "dark", accent: "", background: "", ...cached });
  } catch {
    // No cache: the saved settings apply once loaded.
  }
  const apply = () => {
    const { appearance } = useSettings.getState();
    applyAppearance(appearance);
    try {
      localStorage.setItem(CACHE_KEY, JSON.stringify(appearance));
    } catch {
      // Storage unavailable: only the startup flash comes back.
    }
  };
  void settingsReady.then(apply);
  useSettings.subscribe((state, prev) => {
    if (state.appearance !== prev.appearance) apply();
  });
  lightQuery().addEventListener("change", () => useSettings.getState().appearance.theme === "system" && apply());
}
