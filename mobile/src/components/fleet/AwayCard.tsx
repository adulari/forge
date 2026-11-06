// "While you were away": one card on Fleet after a gap of 30+ minutes, answering what happened to
// the sessions left running without opening each one. Each line opens its session; the × clears
// the card (the last-visit stamp has already moved on, so it does not come back).
import { useQuery } from "@tanstack/react-query";
import { X } from "lucide-react-native";
import React, { useCallback, useEffect, useState } from "react";
import { AppState, Pressable, StyleSheet, Text, View } from "react-native";

import { useAuth } from "../../lib/auth";
import { getBrief } from "../../lib/brief/api";
import {
  awaySince,
  cardTitle,
  headline,
  lineDetail,
  lineState,
  visibleLines,
  worthShowing,
  type Brief,
  type BriefOutcome,
} from "../../lib/brief/core";
import { readLastVisit, stampVisit } from "../../lib/brief/store";
import { isWeb } from "../../lib/platform";
import { useOpenSession } from "../../lib/useScopedFleet";
import { useTokens } from "../../theme/ThemeProvider";
import { space, type StatusDotState } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Card } from "../ds/Card";
import { IconButton } from "../ds/IconButton";
import { StatusDot } from "../ds/StatusDot";

const HEARTBEAT_MS = 5 * 60_000;

function dotFor(outcome: BriefOutcome): StatusDotState {
  switch (outcome) {
    case "waiting":
    case "failed":
      return "waiting";
    case "working":
      return "busy";
    case "interrupted":
      return "idle";
    default:
      return "done";
  }
}

const nowSecs = () => Math.floor(Date.now() / 1000);

export function AwayCard() {
  const tokens = useTokens();
  const { baseUrl } = useAuth();
  const openSession = useOpenSession();
  const [since, setSince] = useState<number | null>(null);
  const [dismissed, setDismissed] = useState(false);

  // Read the previous stamp, decide whether this is a return, then stamp now. Leaving and a slow
  // heartbeat also stamp, so "last visit" means the last time the app was actually in front.
  useEffect(() => {
    if (!baseUrl) return;
    let cancelled = false;
    const arrive = async () => {
      const now = nowSecs();
      const away = awaySince(await readLastVisit(baseUrl), now);
      if (cancelled) return;
      if (away != null) {
        setSince(away);
        setDismissed(false);
      }
      void stampVisit(baseUrl, now);
    };
    const leave = () => void stampVisit(baseUrl);
    void arrive();
    const lifecycle = AppState.addEventListener("change", (state) => {
      if (state === "active") void arrive();
      else leave();
    });
    const onVisibility = () => {
      if (document.visibilityState === "visible") void arrive();
      else leave();
    };
    if (isWeb) document.addEventListener("visibilitychange", onVisibility);
    const heartbeat = setInterval(() => {
      if (AppState.currentState === "active") leave();
    }, HEARTBEAT_MS);
    return () => {
      cancelled = true;
      lifecycle.remove();
      clearInterval(heartbeat);
      if (isWeb) document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [baseUrl]);

  const brief = useQuery({
    queryKey: ["brief", baseUrl, since],
    queryFn: () => getBrief(baseUrl as string, since as number),
    enabled: baseUrl != null && since != null && !dismissed,
    staleTime: Infinity,
    retry: false,
  });
  const dismiss = useCallback(() => setDismissed(true), []);

  if (dismissed || !worthShowing(brief.data)) return null;
  return <AwayCardView brief={brief.data} onOpen={openSession} onDismiss={dismiss} />;
}

export function AwayCardView({ brief: data, onOpen, onDismiss }: { brief: Brief; onOpen: (sessionId: string) => void; onDismiss: () => void }) {
  const tokens = useTokens();
  const { lines, hidden } = visibleLines(data);

  return (
    <Card style={styles.card} testID="away-card">
      <View style={styles.header}>
        <View style={styles.headerText}>
          <Text style={[typeScale.headingBold, { color: tokens.ink }]} accessibilityRole="header">
            {cardTitle(data)}
          </Text>
          <Text style={[typeScale.sub, { color: tokens.ink3 }]}>{headline(data)}</Text>
        </View>
        <IconButton
          icon={<X size={18} strokeWidth={1.75} color={tokens.ink3} />}
          onPress={onDismiss}
          accessibilityLabel="Dismiss summary"
          style={styles.dismiss}
        />
      </View>
      {lines.map((line) => {
        const detail = lineDetail(line);
        return (
          <Pressable
            key={line.id}
            onPress={() => onOpen(line.id)}
            style={styles.line}
            accessibilityRole="button"
            accessibilityLabel={`${line.title}, ${lineState(line, data.now)}`}
          >
            <View style={styles.dot}>
              <StatusDot state={dotFor(line.outcome)} />
            </View>
            <View style={styles.lineText}>
              <Text style={[typeScale.body, { color: tokens.ink }]} numberOfLines={1}>
                {line.title}
              </Text>
              <Text
                style={[typeScale.sub, { color: line.outcome === "failed" || line.outcome === "waiting" ? tokens.danger : tokens.ink3 }]}
                numberOfLines={1}
              >
                {lineState(line, data.now)}
                {detail ? ` · ${detail}` : ""}
              </Text>
            </View>
          </Pressable>
        );
      })}
      {hidden > 0 ? <Text style={[typeScale.sub, styles.more, { color: tokens.ink4 }]}>+{hidden} more in the list below</Text> : null}
    </Card>
  );
}

const styles = StyleSheet.create({
  card: { marginHorizontal: space.space16, marginTop: space.space8, gap: space.space8 },
  header: { flexDirection: "row", alignItems: "flex-start", gap: space.space8 },
  headerText: { flex: 1, gap: space.space2 },
  dismiss: { marginTop: -space.space8, marginRight: -space.space12 },
  line: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingVertical: space.space4 },
  dot: { width: 12, alignItems: "center" },
  lineText: { flex: 1 },
  more: { paddingLeft: 20 },
});
