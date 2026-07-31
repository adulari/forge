import AsyncStorage from "@react-native-async-storage/async-storage";
import { router, usePathname } from "expo-router";
import { clearSharedPayloads, getSharedPayloads, type SharePayload } from "expo-sharing";
import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { AppState, Platform } from "react-native";

import { useToast } from "../components/ds/ToastHost";
import { useAuth } from "./auth";
import {
  decodeIncomingShare,
  textFromSharedPayloads,
  type IncomingShareDraft,
} from "./incomingShareCore";

const STORAGE_KEY = "forge.incomingShare.v1";

interface IncomingShareContextValue {
  pendingShare: IncomingShareDraft | null;
  consumeShare: (id: string) => Promise<void>;
}

const IncomingShareContext = createContext<IncomingShareContextValue>({
  pendingShare: null,
  consumeShare: async () => undefined,
});

function draftFor(text: string): IncomingShareDraft {
  const createdAt = Date.now();
  return {
    id: `share-${createdAt.toString(36)}-${Math.random().toString(36).slice(2, 10)}`,
    text,
    createdAt,
  };
}

export function IncomingShareProvider({ children }: React.PropsWithChildren) {
  const [pendingShare, setPendingShare] = useState<IncomingShareDraft | null>(null);
  const [hydrated, setHydrated] = useState(false);
  const { isPaired } = useAuth();
  const pathname = usePathname();
  const toast = useToast();
  const presentedId = useRef<string | null>(null);
  const pendingShareRef = useRef<IncomingShareDraft | null>(null);
  const receiving = useRef(false);

  useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const raw = await AsyncStorage.getItem(STORAGE_KEY);
        if (!active) return;
        const decoded = decodeIncomingShare(raw);
        pendingShareRef.current = decoded;
        setPendingShare(decoded);
        if (raw && !decoded) await AsyncStorage.removeItem(STORAGE_KEY);
      } catch (error) {
        console.warn("Could not restore content shared into Forge", error);
        toast.show("Couldn't restore previously shared content.", { tone: "danger" });
      } finally {
        if (active) setHydrated(true);
      }
    })();
    return () => {
      active = false;
    };
  }, [toast]);

  const receiveNativeShare = useCallback(async () => {
    if (Platform.OS === "web" || !hydrated || receiving.current) return;
    receiving.current = true;
    try {
      let payloads: SharePayload[];
      try {
        payloads = getSharedPayloads();
      } catch (error) {
        console.warn("Could not read content shared into Forge", error);
        return;
      }
      if (payloads.length === 0) return;

      const text = textFromSharedPayloads(payloads);
      if (!text) {
        clearSharedPayloads();
        toast.show("That shared content is unsupported or larger than 65,536 characters.", {
          tone: "danger",
        });
        return;
      }

      // Do not overwrite the only durable copy of a share that has not been submitted or
      // explicitly discarded. expo-sharing retains the new native payload; consumeShare retries
      // it immediately after the current draft is resolved.
      if (pendingShareRef.current) {
        toast.show("Finish or discard the current shared draft before importing another.", {
          tone: "neutral",
        });
        return;
      }

      const draft = draftFor(text);
      // Persist before acknowledging the native handoff. A process termination must leave one
      // recoverable copy on one side of the boundary.
      await AsyncStorage.setItem(STORAGE_KEY, JSON.stringify(draft));
      pendingShareRef.current = draft;
      setPendingShare(draft);
      clearSharedPayloads();
      presentedId.current = null;
    } catch (error) {
      console.warn("Could not persist content shared into Forge", error);
      toast.show("Couldn't import shared content.", { tone: "danger" });
    } finally {
      receiving.current = false;
    }
  }, [hydrated, toast]);

  useEffect(() => {
    void receiveNativeShare();
    if (Platform.OS === "web") return;
    const subscription = AppState.addEventListener("change", (state) => {
      if (state === "active") void receiveNativeShare();
    });
    return () => subscription.remove();
  }, [receiveNativeShare]);

  useEffect(() => {
    if (!pendingShare || !isPaired || presentedId.current === pendingShare.id) return;
    presentedId.current = pendingShare.id;
    if (pathname === "/new-session") {
      router.setParams({ incomingShare: pendingShare.id });
    } else {
      router.push({
        pathname: "/new-session",
        params: { incomingShare: pendingShare.id },
      });
    }
  }, [isPaired, pathname, pendingShare]);

  const consumeShare = useCallback(async (id: string) => {
    if (pendingShareRef.current?.id !== id) return;
    try {
      await AsyncStorage.removeItem(STORAGE_KEY);
    } catch (error) {
      console.warn("Could not remove content shared into Forge", error);
      toast.show("Couldn't clear the shared draft. Try again.", { tone: "danger" });
      return;
    }
    pendingShareRef.current = null;
    setPendingShare(null);
    presentedId.current = null;
    // A second native share is intentionally left unacknowledged while the current draft exists.
    // Import it now rather than waiting for another foreground transition.
    void receiveNativeShare();
  }, [receiveNativeShare, toast]);

  const value = useMemo(
    () => ({ pendingShare, consumeShare }),
    [consumeShare, pendingShare],
  );
  return <IncomingShareContext.Provider value={value}>{children}</IncomingShareContext.Provider>;
}

export function useIncomingShare(): IncomingShareContextValue {
  return useContext(IncomingShareContext);
}
