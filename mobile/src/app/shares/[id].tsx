// Public read-only replay link: `{service}/shares/<id>#key=…&signing=…`, created on the host by
// `forge anywhere share`. The key material lives only in the fragment, so the replay is fetched
// as sealed bytes and decrypted here (`retrieveReplayShare`).
//
// This screen used to render the prototype MockAnywhereClient's canned share — the same demo
// transcript for every link — and never decrypted anything.
import * as Linking from "expo-linking";
import { useLocalSearchParams } from "expo-router";
import { Flame } from "lucide-react-native";
import { useEffect, useState } from "react";
import { Platform, StyleSheet, Text, View } from "react-native";

import { Badge } from "../../components/ds/Badge";
import { Screen } from "../../components/ds/Screen";
import { type ReplayShare, retrieveReplayShare } from "../../lib/anywhereShare";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type } from "../../theme/typography";

interface ReplayTurn {
  role?: string;
  content?: string;
  model?: string | null;
  tool_calls?: { name?: string }[];
}

interface ReplayDocument {
  summary?: { prompts?: number; messages?: number; models?: string[] };
  turns?: ReplayTurn[];
}

type State =
  | { phase: "loading" }
  | { phase: "ready"; share: ReplayShare<ReplayDocument> }
  | { phase: "error"; message: string };

async function linkHref(): Promise<string | null> {
  if (Platform.OS === "web" && typeof window !== "undefined") return window.location.href;
  return Linking.getInitialURL();
}

export default function ShareViewerScreen() {
  const tokens = useTokens();
  const { id } = useLocalSearchParams<{ id: string }>();
  const [state, setState] = useState<State>({ phase: "loading" });

  useEffect(() => {
    if (!id) return;
    let cancelled = false;
    void (async () => {
      try {
        const href = await linkHref();
        if (!href || !href.includes("#")) throw new Error("The link is missing its key — ask for it again.");
        const share = await retrieveReplayShare<ReplayDocument>({
          serviceUrl: new URL(href).origin,
          shareId: id,
          href,
        });
        if (!cancelled) setState({ phase: "ready", share });
      } catch (error) {
        if (!cancelled) setState({ phase: "error", message: error instanceof Error ? error.message : String(error) });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [id]);

  if (state.phase === "loading") {
    return (
      <Screen contentContainerStyle={styles.content}>
        <Text style={[type.sub, { color: tokens.ink2 }]}>Decrypting on this device… the server only stored sealed bytes.</Text>
      </Screen>
    );
  }
  if (state.phase === "error") {
    return (
      <Screen contentContainerStyle={styles.content}>
        <Text style={[type.bodyBold, { color: tokens.ink }]}>This replay could not be opened</Text>
        <Text style={[type.sub, { color: tokens.danger }]}>{state.message}</Text>
      </Screen>
    );
  }

  const replay = state.share.replay;
  const turns = (replay.turns ?? []).filter((turn) => (turn.content ?? "").trim() || (turn.tool_calls ?? []).length);
  return (
    <Screen scroll contentContainerStyle={styles.content}>
      <View style={styles.header}>
        <Flame size={16} color={tokens.accent} fill={tokens.accent} />
        <Text style={[type.bodyBold, styles.title, { color: tokens.ink }]} numberOfLines={1}>
          Session {state.share.session_id.slice(0, 8)}
        </Text>
        <Badge label="REPLAY · READ-ONLY" tone="outline" />
      </View>
      <Text style={[type.monoMeta, { color: tokens.ink4 }]}>
        {`${replay.summary?.prompts ?? 0} prompts · expires ${new Date(state.share.expires_at_ms).toLocaleDateString()}`}
      </Text>
      {turns.map((turn, index) => (
        <View key={index} style={[styles.turn, { borderColor: tokens.border }]}>
          <Text style={[type.monoMeta, { color: turn.role === "user" ? tokens.accent : tokens.ink3 }]}>
            {turn.role === "user" ? "you" : turn.role === "assistant" ? "forge" : (turn.role ?? "")}
          </Text>
          {(turn.content ?? "").trim() ? (
            <Text selectable style={[type.body, { color: tokens.ink }]}>{turn.content}</Text>
          ) : null}
          {(turn.tool_calls ?? []).map((call, callIndex) => (
            <Text key={callIndex} style={[type.monoMeta, { color: tokens.ink3 }]}>{`▸ ${call.name ?? "tool"}`}</Text>
          ))}
        </View>
      ))}
    </Screen>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space24, paddingBottom: space.space32, gap: space.space12 },
  header: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  title: { flex: 1 },
  turn: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius8, padding: space.space12, gap: space.space4 },
});
