// One-time "Enable notifications?" modal for an installed iOS web app. Safari only grants the
// permission from a tap, so the app asks with its own dialog at launch while the answer is still
// open, and the tap on Enable is the gesture the browser needs. Any other platform renders nothing.
import AsyncStorage from "@react-native-async-storage/async-storage";
import React, { useCallback, useEffect, useState } from "react";

import { useAuth } from "../../lib/auth";
import { pwaPromptDue } from "../../lib/notifyPrefsCore";
import { isIOSWeb, isStandalonePwa } from "../../lib/pwaEnv";
import { enablePush, isPushSupported } from "../../lib/push";
import { ConfirmDialog } from "../ds/ConfirmDialog";
import { useToast } from "../ds/ToastHost";

const DISMISSED_KEY = "forge.pwaNotifyPrompt.v1";

function permission(): "default" | "granted" | "denied" | "unsupported" {
  return typeof Notification === "undefined" ? "unsupported" : Notification.permission;
}

export function EnableNotificationsPrompt() {
  const { baseUrl, isLoading } = useAuth();
  const toast = useToast();
  const [visible, setVisible] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (isLoading || !baseUrl || !isPushSupported()) return;
    let cancelled = false;
    void AsyncStorage.getItem(DISMISSED_KEY)
      .catch(() => "1")
      .then((stored) => {
        if (cancelled) return;
        setVisible(
          pwaPromptDue({ isIOS: isIOSWeb(), standalone: isStandalonePwa(), permission: permission(), dismissed: stored != null }),
        );
      });
    return () => {
      cancelled = true;
    };
  }, [isLoading, baseUrl]);

  const close = useCallback(() => {
    setVisible(false);
    void AsyncStorage.setItem(DISMISSED_KEY, "1").catch(() => undefined);
  }, []);

  const enable = useCallback(async () => {
    if (!baseUrl || busy) return;
    setBusy(true);
    try {
      const state = await enablePush(baseUrl);
      if (state !== "subscribed") toast.show("notifications stay off — you can turn them on in Settings.", { tone: "neutral" });
    } catch {
      toast.show("couldn't enable notifications — try again from Settings.", { tone: "warn" });
    } finally {
      setBusy(false);
      close();
    }
  }, [baseUrl, busy, close, toast]);

  return (
    <ConfirmDialog
      visible={visible}
      title="Enable notifications?"
      message="Forge can tell you when a session finishes, fails or needs your answer, even with this app closed."
      confirmLabel="Enable"
      cancelLabel="Not now"
      loading={busy}
      onConfirm={() => void enable()}
      onCancel={close}
    />
  );
}
