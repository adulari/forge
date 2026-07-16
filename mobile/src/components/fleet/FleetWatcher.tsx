import { AppState, type AppStateStatus } from "react-native";
import { useEffect, useRef } from "react";
import { useQueryClient } from "@tanstack/react-query";

import { useToast } from "../ds/ToastHost";
import { haptics } from "../../lib/haptics";
import { useSessions } from "../../lib/queries";
import type { SessionRow } from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { TWebSocket } from "../../lib/transport";

const DEBOUNCE_MS = 1200;

export function FleetWatcher() {
  const { data } = useSessions();
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  const toast = useToast();
  const previous = useRef<Map<string, SessionRow>>(new Map());
  const foreground = useRef(true);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (!baseUrl) return;
    let active = true;
    let socket: WebSocket | null = null;
    let reconnect: ReturnType<typeof setTimeout> | null = null;
    let attempts = 0;

    const connect = () => {
      if (!active || AppState.currentState !== "active") return;
      const url = new URL(`${baseUrl}/ws/fleet`);
      url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
      socket = new TWebSocket(url.toString());
      socket.onopen = () => { attempts = 0; };
      socket.onmessage = (event) => {
        try {
          const frame = JSON.parse(String(event.data)) as { kind?: unknown; revision?: unknown };
          if (frame.kind === "fleet_changed" && typeof frame.revision === "number") {
            void queryClient.invalidateQueries({ queryKey: ["sessions", baseUrl] });
          }
        } catch {
          // Ignore malformed invalidations; the recovery poll remains authoritative.
        }
      };
      socket.onclose = () => {
        socket = null;
        if (!active || AppState.currentState !== "active") return;
        const delay = Math.min(15_000, 500 * 2 ** attempts++);
        reconnect = setTimeout(connect, delay);
      };
    };

    const lifecycle = AppState.addEventListener("change", (state) => {
      if (state === "active") connect();
      else {
        if (reconnect) clearTimeout(reconnect);
        reconnect = null;
        socket?.close();
        socket = null;
      }
    });
    connect();
    return () => {
      active = false;
      lifecycle.remove();
      if (reconnect) clearTimeout(reconnect);
      socket?.close();
    };
  }, [baseUrl, queryClient]);

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
