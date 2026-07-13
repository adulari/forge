// DESIGN_SYSTEM.md §5.2 Signal — ToastHost: context provider + useToast(),
// stacks Toasts bottom-up, auto-dismiss 3.5s.
import React, { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { StyleSheet, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { isTauri } from "../../lib/platform";
import { notifySupported, notify } from "../../lib/notify";
import { Toast, type ToastData, type ToastTone } from "./Toast";

// Only escalate to a system notification when the window is backgrounded — while Forge
// is focused the in-app toast already did the job (ARCHITECTURE.md §6.1/§6.2).
//
// A Tauri webview does NOT flip `document.visibilityState` on window blur (only on
// minimize), so relying on visibilityState alone means alt-tabbed desktop users get
// neither the toast nor the OS notification. Track real window focus instead:
//   - Tauri: `getCurrentWindow().onFocusChanged` (the actual OS-level focus signal).
//   - Non-Tauri web: `document.visibilityState` (tab hidden) OR `!document.hasFocus()`
//     (tab visible but the browser window itself isn't focused, e.g. alt-tabbed to
//     another app). This is a deliberate widening vs. the old tab-hidden-only check —
//     "I wasn't looking at it" should escalate either way, not just when the tab is gone.
// Native (iOS/Android) has no local-notification story yet (§7), so it stays `false`.
let backgrounded = false;

function isBackgrounded(): boolean {
  return backgrounded;
}

let focusTrackingStarted = false;

function startFocusTracking() {
  if (focusTrackingStarted) return;
  focusTrackingStarted = true;

  if (isTauri) {
    void import("@tauri-apps/api/window").then(({ getCurrentWindow }) => {
      const win = getCurrentWindow();
      void win.isFocused().then((focused) => {
        backgrounded = !focused;
      });
      void win.onFocusChanged(({ payload: focused }) => {
        backgrounded = !focused;
      });
    });
    return;
  }

  if (typeof document === "undefined") return;

  const update = () => {
    backgrounded = document.visibilityState === "hidden" || !document.hasFocus();
  };
  update();
  document.addEventListener("visibilitychange", update);
  window.addEventListener("focus", update);
  window.addEventListener("blur", update);
}

export interface ShowToastOptions {
  tone?: ToastTone;
  /** ms before auto-dismiss. Default 3500 (§5.2 Signal). */
  duration?: number;
}

interface ToastContextValue {
  show: (message: string, options?: ShowToastOptions) => string;
  dismiss: (id: string) => void;
}

const ToastContext = createContext<ToastContextValue | null>(null);

const AUTO_DISMISS_MS = 3500;
let nextId = 0;

export function ToastHost({ children }: { children: React.ReactNode }) {
  const [toasts, setToasts] = useState<ToastData[]>([]);
  const timers = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());

  useEffect(() => {
    startFocusTracking();
    const activeTimers = timers.current;
    return () => {
      for (const timer of activeTimers.values()) clearTimeout(timer);
      activeTimers.clear();
    };
  }, []);

  const dismiss = useCallback((id: string) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
    const timer = timers.current.get(id);
    if (timer) {
      clearTimeout(timer);
      timers.current.delete(id);
    }
  }, []);

  const show = useCallback(
    (message: string, options?: ShowToastOptions) => {
      const id = `toast-${nextId++}`;
      setToasts((prev) => [...prev, { id, message, tone: options?.tone }]);
      const timer = setTimeout(() => dismiss(id), options?.duration ?? AUTO_DISMISS_MS);
      timers.current.set(id, timer);
      if (isBackgrounded() && notifySupported()) {
        void notify("Forge", message);
      }
      return id;
    },
    [dismiss],
  );

  const value = useMemo(() => ({ show, dismiss }), [show, dismiss]);

  return (
    <ToastContext.Provider value={value}>
      {children}
      <SafeAreaView style={[styles.host, { pointerEvents: "box-none" }]} edges={["bottom"]}>
        <View style={[styles.stack, { pointerEvents: "box-none" }]}>
          {toasts.map((t) => (
            <Toast key={t.id} toast={t} onDismiss={dismiss} />
          ))}
        </View>
      </SafeAreaView>
    </ToastContext.Provider>
  );
}

export function useToast(): ToastContextValue {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToast must be used within a ToastHost");
  return ctx;
}

const styles = StyleSheet.create({
  host: { position: "absolute", left: 0, right: 0, bottom: 0 },
  stack: { justifyContent: "flex-end" },
});
