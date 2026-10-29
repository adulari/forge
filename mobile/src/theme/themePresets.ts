import type { ThemeDocument } from "./themeDocument";

export const DEFAULT_THEME_ID = "forge";

export const THEME_PRESETS: readonly ThemeDocument[] = [
  {
    id: "midnight",
    name: "Midnight",
    dark: { bg0: "#080B14", bg1: "#0B0F1C", bg2: "#0D1220", bg3: "#10162A", ink: "#E8ECF8", ink2: "#97A1BF", ink3: "#6B7595", accent: "#6EA8FF", onAccent: "#06101F", success: "#5FC48A", danger: "#F0706B", warn: "#E0B35A" },
    light: { bg0: "#F2F5FB", bg1: "#E8EDF7", bg2: "#FFFFFF", bg3: "#F6F8FC", ink: "#141A2B", ink2: "#4D5879", ink3: "#66708F", accent: "#1F5FD1", onAccent: "#FFFFFF", success: "#2F7D55", danger: "#C0392F", warn: "#8A6A1C" },
  },
  {
    id: "graphite",
    name: "Graphite",
    dark: { bg0: "#101010", bg1: "#151515", bg2: "#181818", bg3: "#1D1D1D", ink: "#EDEDED", ink2: "#A0A0A0", ink3: "#7A7A7A", accent: "#E6E6E6", onAccent: "#101010", success: "#6CBF86", danger: "#E5736F", warn: "#D6AE5C" },
    light: { bg0: "#F4F4F4", bg1: "#EAEAEA", bg2: "#FFFFFF", bg3: "#F8F8F8", ink: "#161616", ink2: "#4F4F4F", ink3: "#696969", accent: "#2A2A2A", onAccent: "#FFFFFF", success: "#2F7D4B", danger: "#B93A34", warn: "#8A6A1C" },
  },
  {
    id: "forest",
    name: "Forest",
    dark: { bg0: "#0A0F0C", bg1: "#0E140F", bg2: "#101811", bg3: "#141D16", ink: "#E6F0E8", ink2: "#97AB9C", ink3: "#6E8574", accent: "#6FCF8E", onAccent: "#06140B", success: "#6FCF8E", danger: "#E5736F", warn: "#D9B055" },
    light: { bg0: "#F1F5F1", bg1: "#E6EDE6", bg2: "#FFFFFF", bg3: "#F6F9F6", ink: "#14201A", ink2: "#4A5E50", ink3: "#62786A", accent: "#1F7A45", onAccent: "#FFFFFF", success: "#1F7A45", danger: "#B93A34", warn: "#86661A" },
  },
  {
    id: "nord",
    name: "Nord",
    dark: { bg0: "#242933", bg1: "#2B313D", bg2: "#2E3440", bg3: "#353C4A", ink: "#ECEFF4", ink2: "#B4BDCE", ink3: "#8E99AE", accent: "#88C0D0", onAccent: "#1D2530", success: "#A3BE8C", danger: "#E27D86", warn: "#EBCB8B" },
    light: { bg0: "#ECEFF4", bg1: "#E5E9F0", bg2: "#FFFFFF", bg3: "#F3F5F9", ink: "#2E3440", ink2: "#4C566A", ink3: "#5F6A80", accent: "#2F6F8F", onAccent: "#FFFFFF", success: "#4E7A3A", danger: "#B2404B", warn: "#85641A" },
  },
  {
    id: "solarized",
    name: "Solarized",
    dark: { bg0: "#00212B", bg1: "#002B36", bg2: "#073642", bg3: "#0C3E4B", ink: "#EEE8D5", ink2: "#A8B5B2", ink3: "#8195A0", accent: "#B58900", onAccent: "#00212B", success: "#85990A", danger: "#E8504D", warn: "#CB8F1F" },
    light: { bg0: "#FDF6E3", bg1: "#EEE8D5", bg2: "#FFFBEF", bg3: "#F7F0DC", ink: "#073642", ink2: "#4A6068", ink3: "#5E7780", accent: "#9A6F00", onAccent: "#FFFFFF", success: "#5F7000", danger: "#C0322F", warn: "#8A6200" },
  },
  {
    id: "rose",
    name: "Rose",
    dark: { bg0: "#120A0E", bg1: "#180E13", bg2: "#1B1015", bg3: "#21151B", ink: "#F6E9EE", ink2: "#BFA3AE", ink3: "#957A86", accent: "#F28FB0", onAccent: "#240A14", success: "#6FC48C", danger: "#F0706B", warn: "#DDB05A" },
    light: { bg0: "#FBF3F5", bg1: "#F4E6EA", bg2: "#FFFFFF", bg3: "#FCF7F8", ink: "#2A1219", ink2: "#6B4A55", ink3: "#80606B", accent: "#B8325F", onAccent: "#FFFFFF", success: "#2F7D55", danger: "#B93A34", warn: "#86661A" },
  },
];
