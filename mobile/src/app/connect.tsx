// Pairing screen — storage plumbing only for B0 (nav shell + auth wiring). Full UI/UX
// (QR scan, distinct copy per failure class) lands in Batch 1 (BUILD_PLAN §7, W1).
import { router } from "expo-router";
import React, { useState } from "react";

import {
  ErrorText,
  PrimaryButton,
  Screen,
  SearchInput,
  SectionTitle,
} from "../components/ui";
import { type ConnectTestState, useAuth } from "../lib/auth";

const STATE_COPY: Record<Exclude<ConnectTestState, "idle" | "testing" | "ok">, string> = {
  "bad-token": "Pairing invalid — re-scan the QR code from `forge serve`.",
  unreachable:
    "Server unreachable. Use --local + Tailscale/VPN, or --anywhere for a public tunnel.",
  "server-error": "The server returned an error. Check `forge serve` logs.",
};

export default function ConnectScreen() {
  const { pair } = useAuth();
  const [url, setUrl] = useState("");
  const [state, setState] = useState<ConnectTestState>("idle");
  const [error, setError] = useState<string | null>(null);

  const onConnect = async () => {
    setState("testing");
    setError(null);
    try {
      await pair(url);
      router.replace("/(tabs)");
    } catch (err) {
      setState("bad-token");
      setError(err instanceof Error ? err.message : "Could not pair");
    }
  };

  return (
    <Screen keyboardAvoiding contentContainerClassName="gap-10 pt-16">

      <SectionTitle>Pair with Forge</SectionTitle>
      <SearchInput
        value={url}
        onChangeText={setUrl}
        placeholder="connect: URL from `forge serve`"
        autoCapitalize="none"
        autoCorrect={false}
        returnKeyType="go"
        onSubmitEditing={onConnect}
      />
      <PrimaryButton
        label={state === "testing" ? "Connecting…" : "Connect"}
        onPress={onConnect}
        loading={state === "testing"}
        disabled={url.trim().length === 0}
      />
      {error && state !== "idle" && state !== "testing" && state !== "ok" ? (
        <ErrorText message={STATE_COPY[state] ?? error} />
      ) : null}
    </Screen>
  );
}
