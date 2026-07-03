// Forge brand — Catppuccin-Mocha-ish dark base with forge-orange ember accent.
export const C = {
  base: "#1e1e2e",
  mantle: "#181825",
  crust: "#11111b",
  surface: "#313244",
  overlay: "#45475a",
  text: "#cdd6f4",
  subtext: "#a6adc8",
  muted: "#6c7086",
  orange: "#f5a97f",
  ember: "#fab387",
  emberDeep: "#e08a52",
  lavender: "#b4befe",
  green: "#a6e3a1",
  red: "#f38ba8",
  yellow: "#f9e2af",
  blue: "#89b4fa",
  teal: "#94e2d5",
} as const;

export const FONT = "JetBrains Mono";

export const glow = (color: string, strength = 1) =>
  `0 0 ${8 * strength}px ${color}, 0 0 ${20 * strength}px ${color}66, 0 0 ${40 * strength}px ${color}33`;
