// DESIGN_SYSTEM.md §5.2 Signal — ToastHost: context provider + useToast(),
// stacks Toasts bottom-up, auto-dismiss 3.5s.
import React, { createContext, useCallback, useContext, useMemo, useRef, useState } from "react";
import { StyleSheet, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { notifySupported, notify } from "../../lib/notify";
import { Toast, type ToastData, type ToastTone } from "./Toast";

// Only escalate to a system notification when the window/tab is backgrounded — while
// Forge is focused the in-app toast already did the job (ARCHITECTURE.md §6.1/§6.2).
function isBackgrounded(): boolean {
  return typeof document !== "undefined" && document.visibilityState === "hidden";
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
