// Pairing screen (BUILD_PLAN §6 "Connect", Batch 1 W1). Paste the `connect:` URL that
// `forge serve` prints (or QR-scans as a fallback — camera scanning is a marked stub, see
// below), live-validate it against the real daemon via `testConnection()`, then persist via
// `pair()` and route into the app. A manual host+token fallback covers cases where the
// printed URL got mangled in transit.
import { router } from "expo-router";
import React, { useCallback, useState } from "react";
import { Platform, Pressable, Text, View } from "react-native";

import {
  Card,
  Chip,
  EntranceView,
  ErrorText,
  PrimaryButton,
  Screen,
  SearchInput,
  Segmented,
  SectionTitle,
} from "../components/ui";
import { type ConnectTestState, parseConnectUrl, useAuth } from "../lib/auth";

// Distinct copy per failure class (UI_RULES.md #14) — matches the language used in
// BUILD_PLAN §1.1 (exposure modes) so the fix is always actionable, not just descriptive.
const STATE_COPY: Record<Exclude<ConnectTestState, "idle" | "testing" | "ok">, string> = {
  "bad-token": "Pairing invalid — re-scan or re-copy the connect URL from `forge serve`.",
  unreachable:
    "Server unreachable. On the same network? Try --local + Tailscale/VPN. Remote? Use --anywhere for a public tunnel.",
  "server-error": "The server returned an error. Check the `forge serve` logs on the host.",
};

function successHaptic() {
  if (Platform.OS === "web") return;
  import("expo-haptics")
    .then((Haptics) =>
      Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success),
    )
    .catch(() => {});
}

export default function ConnectScreen() {
  const { pair, testConnection } = useAuth();

  const [url, setUrl] = useState("");
  const [testState, setTestState] = useState<ConnectTestState>("idle");
  const [formatError, setFormatError] = useState<string | null>(null);

  const [manualOpen, setManualOpen] = useState(false);
  const [manualScheme, setManualScheme] = useState<"https" | "http">("https");
  const [manualHost, setManualHost] = useState("");
  const [manualToken, setManualToken] = useState("");

  const manualCandidate =
    manualHost.trim().length > 0 && manualToken.trim().length > 0
      ? `${manualScheme}://${manualHost.trim()}/${manualToken.trim()}`
      : "";

  const busy = testState === "testing";

  const attemptConnect = useCallback(
    async (candidate: string) => {
      setFormatError(null);
      const parsed = parseConnectUrl(candidate);
      if (!parsed) {
        setTestState("idle");
        setFormatError(
          "That doesn't look like a Forge connect URL. Paste it exactly as `forge serve` printed it.",
        );
        return;
      }
      setTestState("testing");
      const result = await testConnection(parsed.baseUrl);
      setTestState(result);
      if (result === "ok") {
        await pair(candidate);
        successHaptic();
        router.replace("/(tabs)");
      }
    },
    [pair, testConnection],
  );

  const onConnect = () => attemptConnect(url);
  const onManualConnect = () => attemptConnect(manualCandidate);

  return (
    <Screen keyboardAvoiding contentContainerClassName="gap-16 pt-24">
      <EntranceView index={0}>
        <View className="gap-4">
          <Text className="text-accent text-[16px] font-bold">⚒ Connect to Forge</Text>
          <Text className="text-dim text-[13px]">
            Paste the connect URL printed by `forge serve` in your terminal — the same one
            rendered there as a QR code.
          </Text>
        </View>
      </EntranceView>

      <EntranceView index={1}>
        <Card variant="feature" className="gap-10">
          <SectionTitle>Connect URL</SectionTitle>
          <SearchInput
            value={url}
            onChangeText={(t) => {
              setUrl(t);
              setFormatError(null);
            }}
            placeholder="connect://host:7420/<token>"
            autoCapitalize="none"
            autoCorrect={false}
            returnKeyType="go"
            editable={!busy}
            onSubmitEditing={onConnect}
          />
          <Chip label="⌗ Scan QR (coming soon)" disabled />
          <PrimaryButton
            label={busy ? "Connecting…" : "Connect"}
            onPress={onConnect}
            loading={busy}
            disabled={busy || url.trim().length === 0}
          />
        </Card>
      </EntranceView>

      {formatError ? <ErrorText message={formatError} /> : null}
      {testState !== "idle" && testState !== "testing" && testState !== "ok" ? (
        <ErrorText message={STATE_COPY[testState]} onRetry={onConnect} />
      ) : null}
      {testState === "ok" ? (
        <Text className="text-ok text-[14px] text-center">Connected — opening Forge…</Text>
      ) : null}

      <EntranceView index={2}>
        <Pressable
          onPress={() => setManualOpen((v) => !v)}
          hitSlop={8}
          style={{ minHeight: 44, justifyContent: "center" }}
        >
          <Text className="text-dim text-[13px] text-center underline">
            {manualOpen ? "Hide manual entry" : "Enter host + token manually"}
          </Text>
        </Pressable>
      </EntranceView>

      {manualOpen ? (
        <EntranceView index={3}>
          <Card className="gap-10">
            <SectionTitle>Manual pairing</SectionTitle>
            <Segmented
              options={[
                { key: "https", label: "https (--anywhere)" },
                { key: "http", label: "http (--local)" },
              ]}
              value={manualScheme}
              onChange={(k) => setManualScheme(k as "https" | "http")}
            />
            <SearchInput
              value={manualHost}
              onChangeText={setManualHost}
              placeholder="host:port — e.g. 127.0.0.1:7420"
              autoCapitalize="none"
              autoCorrect={false}
              editable={!busy}
            />
            <SearchInput
              value={manualToken}
              onChangeText={setManualToken}
              placeholder="daemon token (16-64 hex chars)"
              autoCapitalize="none"
              autoCorrect={false}
              editable={!busy}
            />
            <PrimaryButton
              label={busy ? "Connecting…" : "Connect"}
              onPress={onManualConnect}
              loading={busy}
              disabled={busy || manualCandidate.length === 0}
            />
          </Card>
        </EntranceView>
      ) : null}
    </Screen>
  );
}
