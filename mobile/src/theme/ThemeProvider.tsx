// Light/dark/system theme selection, persisted across launches.
import AsyncStorage from "@react-native-async-storage/async-storage";
import { StatusBar } from "expo-status-bar";
import React, { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import { Appearance, Platform, useColorScheme as useSystemColorScheme } from "react-native";

import { applyPalette, parseThemeDocument, type ThemeDocument } from "./themeDocument";
import { DEFAULT_THEME_ID, THEME_PRESETS } from "./themePresets";
import { type ColorTokens, darkTokens, lightTokens } from "./tokens";

export type ThemePreference = "light" | "dark" | "system";
export type ThemeScheme = "light" | "dark";

const STORAGE_KEY = "forge.theme";
const THEME_DOC_KEY = "forge.themeDoc";
const MAX_CUSTOM_THEMES = 12;

interface ThemeContextValue {
  /** Resolved scheme (system preference already applied). */
  scheme: ThemeScheme;
  /** Raw user preference, including "system". */
  preference: ThemePreference;
  tokens: ColorTokens;
  setScheme: (pref: ThemePreference) => void;
  /** "forge" (the built-in), a preset id, or `custom:<slug>`. */
  themeId: string;
  themes: ThemeDocument[];
  activeDocument: ThemeDocument | null;
  setThemeId: (id: string) => void;
  /** Throws a readable Error when the text is not a valid theme. */
  importTheme: (text: string) => ThemeDocument;
  removeTheme: (id: string) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const systemScheme = useSystemColorScheme();
  const [preference, setPreference] = useState<ThemePreference>("system");
  const [loaded, setLoaded] = useState(false);
  const [themeId, setThemeIdState] = useState(DEFAULT_THEME_ID);
  const [custom, setCustom] = useState<ThemeDocument[]>([]);

  useEffect(() => {
    let cancelled = false;
    Promise.all([AsyncStorage.getItem(STORAGE_KEY), AsyncStorage.getItem(THEME_DOC_KEY)])
      .then(([stored, themeDoc]) => {
        if (cancelled) return;
        if (stored === "light" || stored === "dark" || stored === "system") {
          setPreference(stored);
        }
        if (themeDoc) {
          const parsed = JSON.parse(themeDoc) as { active?: unknown; custom?: unknown };
          const docs = (Array.isArray(parsed.custom) ? parsed.custom : []).flatMap((entry) => {
            try {
              return [parseThemeDocument(JSON.stringify(entry))];
            } catch {
              return [];
            }
          });
          setCustom(docs);
          if (typeof parsed.active === "string") setThemeIdState(parsed.active);
        }
      })
      .catch(() => undefined)
      .finally(() => {
        if (!cancelled) setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const setScheme = useCallback((pref: ThemePreference) => {
    setPreference(pref);
    void AsyncStorage.setItem(STORAGE_KEY, pref).catch(() => undefined);
  }, []);

  const persistTheme = useCallback((active: string, docs: ThemeDocument[]) => {
    void AsyncStorage.setItem(THEME_DOC_KEY, JSON.stringify({ active, custom: docs })).catch(() => undefined);
  }, []);

  const themes = useMemo(() => [...THEME_PRESETS, ...custom], [custom]);
  const activeDocument = themes.find((doc) => doc.id === themeId) ?? null;

  const setThemeId = useCallback((id: string) => {
    setThemeIdState(id);
    persistTheme(id, custom);
  }, [custom, persistTheme]);

  const importTheme = useCallback((text: string) => {
    const doc = parseThemeDocument(text);
    if (THEME_PRESETS.some((preset) => preset.id === doc.id.slice("custom:".length))) {
      doc.id = `${doc.id}-2`;
    }
    const next = [...custom.filter((existing) => existing.id !== doc.id), doc].slice(-MAX_CUSTOM_THEMES);
    setCustom(next);
    setThemeIdState(doc.id);
    persistTheme(doc.id, next);
    return doc;
  }, [custom, persistTheme]);

  const removeTheme = useCallback((id: string) => {
    const next = custom.filter((doc) => doc.id !== id);
    const active = themeId === id ? DEFAULT_THEME_ID : themeId;
    setCustom(next);
    setThemeIdState(active);
    persistTheme(active, next);
  }, [custom, themeId, persistTheme]);

  const scheme: ThemeScheme = preference === "system" ? (systemScheme === "light" ? "light" : "dark") : preference;
  const baseTokens = scheme === "light" ? lightTokens : darkTokens;
  const tokens = useMemo(
    () => (activeDocument ? applyPalette(baseTokens, activeDocument[scheme], scheme) : baseTokens),
    [activeDocument, baseTokens, scheme],
  );

  // Web/desktop chrome: `theme-color` paints the mobile-browser address bar and the
  // installed-PWA title bar, `color-scheme` is what makes the *browser's own* widgets —
  // scrollbars, form controls, the canvas behind an overscroll — follow the in-app pick
  // instead of the OS setting. Both are the web half of the native-chrome sync below.
  useEffect(() => {
    if (Platform.OS !== "web" || typeof document === "undefined") return;
    document.querySelector('meta[name="theme-color"]')?.setAttribute("content", tokens.bg1);
    document.documentElement.style.setProperty("--forge-focus", tokens.focusRing);
    document.documentElement.style.setProperty("color-scheme", scheme);
  }, [tokens.focusRing, tokens.bg1, scheme]);

  // Native chrome resolves against the OS trait collection, not this provider, so an explicit
  // in-app override used to stop at the app's own pixels: system-light + Forge-dark left the
  // iOS status-bar glyphs drawing dark-on-dark over the dark scheme's `bg1` (the clock and
  // battery became unreadable) and NativeTabs' systemDefault blur rendering a light frosted
  // bar under a fully dark app. `setColorScheme` is the only hook that
  // reaches those surfaces — it writes `overrideUserInterfaceStyle` on every UIWindow on iOS
  // (AppCompat night mode on Android), which also fixes keyboard appearance and native alerts.
  // `"unspecified"` (not null — that is not in the RN enum) hands control back to the OS for the
  // "system" preference. Guarded off web: react-native-web's Appearance shim is read-only and
  // has no setColorScheme at all, and the effect above covers that surface instead.
  useEffect(() => {
    if (Platform.OS === "web") return;
    Appearance.setColorScheme(preference === "system" ? "unspecified" : preference);
  }, [preference]);

  const value = useMemo<ThemeContextValue>(
    () => ({ scheme, preference, tokens, setScheme, themeId, themes, activeDocument, setThemeId, importTheme, removeTheme }),
    [scheme, preference, tokens, setScheme, themeId, themes, activeDocument, setThemeId, importTheme, removeTheme],
  );

  // Hold rendering until the persisted preference has loaded so the app never
  // paints the system-default theme for one frame and then flashes to a saved
  // override — dark is brand-primary, but a saved "light" pick must not flicker.
  if (!loaded) return null;

  return (
    <ThemeContext.Provider value={value}>
      {/* Belt-and-braces over setColorScheme: Info.plist pins UIStatusBarStyleDefault with
          UIViewControllerBasedStatusBarAppearance=false, so on iOS the bar style is whatever
          RN last set app-wide rather than something each screen re-derives. Naming it here is
          also what drives Android's light-status-bar-icons flag under edge-to-edge. Renders
          null on web (expo-status-bar ships a no-op web build). */}
      <StatusBar style={scheme === "dark" ? "light" : "dark"} />
      {children}
    </ThemeContext.Provider>
  );
}

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error("useTheme must be used within a ThemeProvider");
  return ctx;
}

export function useTokens(): ColorTokens {
  return useTheme().tokens;
}
