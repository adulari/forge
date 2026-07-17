// Light/dark/system theme selection, persisted across launches.
import AsyncStorage from "@react-native-async-storage/async-storage";
import React, { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import { Platform, useColorScheme as useSystemColorScheme } from "react-native";

import { type ColorTokens, darkTokens, lightTokens } from "./tokens";

export type ThemePreference = "light" | "dark" | "system";
export type ThemeScheme = "light" | "dark";

const STORAGE_KEY = "forge.theme";

interface ThemeContextValue {
  /** Resolved scheme (system preference already applied). */
  scheme: ThemeScheme;
  /** Raw user preference, including "system". */
  preference: ThemePreference;
  tokens: ColorTokens;
  setScheme: (pref: ThemePreference) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const systemScheme = useSystemColorScheme();
  const [preference, setPreference] = useState<ThemePreference>("system");
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    let cancelled = false;
    AsyncStorage.getItem(STORAGE_KEY)
      .then((stored) => {
        if (cancelled) return;
        if (stored === "light" || stored === "dark" || stored === "system") {
          setPreference(stored);
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

  const scheme: ThemeScheme = preference === "system" ? (systemScheme === "light" ? "light" : "dark") : preference;
  const tokens = scheme === "light" ? lightTokens : darkTokens;

  useEffect(() => {
    if (Platform.OS !== "web" || typeof document === "undefined") return;
    document.querySelector('meta[name="theme-color"]')?.setAttribute("content", tokens.bg1);
    document.documentElement.style.setProperty("--forge-focus", tokens.focusRing);
  }, [tokens.focusRing, tokens.bg1]);

  const value = useMemo<ThemeContextValue>(
    () => ({ scheme, preference, tokens, setScheme }),
    [scheme, preference, tokens, setScheme],
  );

  // Hold rendering until the persisted preference has loaded so the app never
  // paints the system-default theme for one frame and then flashes to a saved
  // override — dark is brand-primary, but a saved "light" pick must not flicker.
  if (!loaded) return null;

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error("useTheme must be used within a ThemeProvider");
  return ctx;
}

export function useTokens(): ColorTokens {
  return useTheme().tokens;
}
