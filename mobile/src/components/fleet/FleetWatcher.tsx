import { AppState, type AppStateStatus } from "react-native";
import { useEffect, useRef } from "react";

import { useToast } from "../ds/ToastHost";
import { haptics } from "../../lib/haptics";
import { useSessions } from "../../lib/queries";
import type { SessionRow } from "../../lib/api";

const DEBOUNCE_MS = 1200;

export function FleetWatcher() {
  const { data } = useSessions();
  const toast = useToast();
  const previous = useRef<Map<string, SessionRow>>(new Map());
  const foreground = useRef(true);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const onState = (next: AppStateStatus) => {
      foreground.current = next === "active";
      if (foreground.current) previous.current = new Map();
    };
    const sub = AppState.addEventListener("change", onState);
    return () => {
      sub.remove();
      if (timer.current) clearTimeout(timer.current);
    };
  }, []);

  useEffect(() => {
    if (!data || !foreground.current) return;
    const next = new Map(data.map((row) => [row.id, row]));
    const old = previous.current;
    if (old.size > 0) {
      const waiting = data.some((row) => row.waiting && !old.get(row.id)?.waiting);
      const finished = data.some((row) => !row.busy && old.get(row.id)?.busy);
      if ((waiting || finished) && !timer.current) {
        timer.current = setTimeout(() => {
          timer.current = null;
          if (waiting) {
            toast.show("another session needs your attention", { tone: "warn" });
          } else {
            toast.show("another session finished", { tone: "success" });
          }
          haptics.select();
        }, DEBOUNCE_MS);
      }
    }
    previous.current = next;
  }, [data, toast]);

  return null;
}
