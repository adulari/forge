// Forge Anywhere — public share viewer (mobile.dc.html "AW Share Viewer", lines
// 796-833). Deliberately NO auth/signed-in guard and NO SettingsShell — this route is
// reachable by anyone with the link, no Forge account required.
import { useLocalSearchParams } from "expo-router";
import { Check, Flame } from "lucide-react-native";
import React, { useEffect, useState } from "react";
import { Platform, StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import { Badge } from "../../components/ds/Badge";
import { Screen } from "../../components/ds/Screen";
import { anywhereClient } from "../../lib/anywhere/store";
import type { ShareFetchResult, ShareRetrievalError } from "../../lib/anywhere/types";
import { useEmberdot } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type } from "../../theme/typography";

const ERROR_COPY: Record<ShareRetrievalError, string> = {
  "key-fragment-missing": "Key fragment missing — the link was truncated. Ask for it again.",
  corrupted: "Data corrupted — decryption failed integrity checks",
  expired: "Expired · content deleted from the relay",
  revoked: "Revoked by the owner",
  "service-unavailable": "Service unavailable — try again shortly",
};

function errorDotColor(error: ShareRetrievalError, tokens: ReturnType<typeof useTokens>): string {
  switch (error) {
    case "key-fragment-missing":
    case "corrupted":
      return tokens.danger;
    case "expired":
    case "revoked":
      return tokens.ink4;
    case "service-unavailable":
      return tokens.warn;
    default: {
      const _exhaustive: never = error;
      return _exhaustive;
    }
  }
}

// The decryption key lives in the URL fragment (`#k=...`), which is never sent to the
// server (types.ts's ReplayShare doc comment). Fragments only survive on web — native
// deep links have no `location.hash` concept, and AnywhereClient.fetchShare's signature
// takes no key parameter for the mock backend anyway, so this check is a web-only,
// best-effort stand-in for a real client-side decryption step.
function hasKeyFragment(): boolean {
  if (Platform.OS !== "web" || typeof window === "undefined") return true;
  return window.location.hash.length > 2;
}

export default function ShareViewerScreen() {
  const tokens = useTokens();
  const { id } = useLocalSearchParams<{ id: string }>();
  const { dotStyle } = useEmberdot("busy");
  const [result, setResult] = useState<ShareFetchResult | null>(null);

  useEffect(() => {
    if (!id) return;
    if (!hasKeyFragment()) {
      setResult({ ok: false, error: "key-fragment-missing" });
      return;
    }
    let cancelled = false;
    void anywhereClient.fetchShare(id).then((r) => {
      if (!cancelled) setResult(r);
    });
    return () => {
      cancelled = true;
    };
  }, [id]);

  return (
    <Screen scroll contentContainerStyle={styles.content} edges={["top", "left", "right", "bottom"]}>
      {result == null ? (
        <View style={styles.loading}>
          <Animated.View style={[styles.dot, dotStyle, { backgroundColor: tokens.accent }]} />
          <Text style={[type.sub, { color: tokens.ink2 }]}>
            Decrypting in your browser… the server only stored sealed bytes
          </Text>
        </View>
      ) : result.ok ? (
        <View>
          <View style={styles.header}>
            <Flame size={16} color={tokens.accent} fill={tokens.accent} />
            <Text style={[type.bodyBold, styles.title, { color: tokens.ink }]} numberOfLines={1}>
              {result.payload.sessionTitle}
            </Text>
            <Badge label="REPLAY · READ-ONLY" tone="outline" />
          </View>
          <Text style={[type.monoMeta, styles.meta, { color: tokens.ink4 }]}>shared replay · read-only · no sign-in</Text>

          <View style={styles.section}>
            <Text style={[type.section, { color: tokens.ink4 }]}>ready — transcript excerpt</Text>
            <Text style={[type.body, styles.transcript, { color: tokens.ink }]}>{result.payload.transcript}</Text>
            {/* ShareViewerPayload only carries a flat transcript string today — no
                structured tool-call list — so this single illustrative row (matching the
                comp's "✓ edit …" tool-call style) stands in until the relay returns
                structured turns; it renders once per share rather than per real event. */}
            <View style={styles.toolCallRow}>
              <View style={[styles.toolCallGlyph, { backgroundColor: tokens.successBg }]}>
                <Check size={9} strokeWidth={3} color={tokens.success} />
              </View>
              <Text style={[type.monoMeta, styles.toolCallText, { color: tokens.ink2 }]} numberOfLines={1}>
                replay ready
              </Text>
            </View>
            <Text style={[type.meta, styles.note, { color: tokens.ink4 }]}>
              Timeline scrubbing only. No composer, no live session control, no browsing beyond
              this replay.
            </Text>
          </View>
        </View>
      ) : (
        <View style={styles.section}>
          <View style={styles.errorRow}>
            <View style={[styles.dot, { backgroundColor: errorDotColor(result.error, tokens) }]} />
            <Text style={[type.sub, styles.errorText, { color: tokens.ink2 }]}>{ERROR_COPY[result.error]}</Text>
          </View>
        </View>
      )}

      <View style={styles.footer}>
        <Text style={[type.meta, styles.footerText, { color: tokens.ink4 }]}>
          Replayed with Forge · forge is free and open source
        </Text>
      </View>
    </Screen>
  );
}

const styles = StyleSheet.create({
  content: { flexGrow: 1, paddingTop: space.space12, paddingBottom: space.space24 },
  loading: { flexDirection: "row", alignItems: "center", gap: space.space8, marginTop: space.space24 },
  dot: { width: 8, height: 8, borderRadius: 4 },
  header: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  title: { flex: 1 },
  meta: { marginLeft: 24 },
  section: { marginTop: space.space24 },
  transcript: { marginTop: space.space12, lineHeight: 20 },
  toolCallRow: { flexDirection: "row", alignItems: "center", gap: space.space8, marginTop: space.space8 },
  toolCallGlyph: { width: 16, height: 16, borderRadius: radii.radius8, alignItems: "center", justifyContent: "center" },
  toolCallText: { flex: 1 },
  note: { marginTop: space.space12 },
  errorRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: 11 },
  errorText: { flex: 1 },
  footer: { flex: 1, justifyContent: "flex-end", alignItems: "center", paddingTop: space.space24 },
  footerText: { textAlign: "center" },
});
