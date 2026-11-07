// "Turn endings": how this session's recent turns ended and what to do about the ones that
// stopped short, plus the host-wide counts for the last week. Opened from the turn-stopped
// banner. The host supplies the plain labels and advice; nothing here prints a raw reason name.
import React from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";

import { useSessionTurns, useTurnsSummary } from "../../lib/queries";
import { countsLine, endedAgo } from "../../lib/turnEnd";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { ListRow } from "../ds/ListRow";
import { Sheet } from "../ds/Sheet";

const SUMMARY_DAYS = 7;

export function TurnEndingsSheet({
  visible,
  onClose,
  sessionId,
  onContinue,
}: {
  visible: boolean;
  onClose: () => void;
  sessionId: string;
  /** Sends `continue`; omitted when the session is not in a state to take it. */
  onContinue?: () => void;
}) {
  const tokens = useTokens();
  const session = useSessionTurns(sessionId, visible);
  const summary = useTurnsSummary(SUMMARY_DAYS, visible);
  const now = Math.floor(Date.now() / 1000);
  const stopped = (session.data?.turns ?? []).filter((turn) => !turn.finished);
  const newest = stopped[0];

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Turn endings" snapPoints={[0.7]}>
      <ScrollView contentContainerStyle={styles.content} keyboardShouldPersistTaps="handled">
        <Text style={[typeScale.heading, { color: tokens.ink }]}>Turn endings</Text>
        {session.error ? (
          <Text style={[typeScale.sub, { color: tokens.danger }]}>
            This host cannot list turn endings yet. Update it to see them here.
          </Text>
        ) : (
          <>
            <View style={styles.block}>
              <Text style={[typeScale.meta, { color: tokens.ink3 }]}>This session</Text>
              <Text style={[typeScale.sub, { color: tokens.ink2 }]}>
                {session.isLoading ? "Loading…" : countsLine(session.data?.counts ?? [])}
              </Text>
              <Text style={[typeScale.meta, { color: tokens.ink3 }]}>All sessions, last {SUMMARY_DAYS} days</Text>
              <Text style={[typeScale.sub, { color: tokens.ink2 }]}>
                {summary.isLoading ? "Loading…" : summary.error ? "Not available." : countsLine(summary.data?.counts ?? [])}
              </Text>
            </View>
            {newest && onContinue && newest.resumable ? (
              <Button
                label="Continue"
                onPress={() => {
                  onContinue();
                  onClose();
                }}
                testID="turn-endings-continue"
              />
            ) : null}
            {stopped.length === 0 && !session.isLoading ? (
              <Text style={[typeScale.sub, { color: tokens.ink3 }]}>Every recent turn in this session finished.</Text>
            ) : (
              stopped.map((turn, index) => (
                <ListRow
                  key={turn.id}
                  title={`${turn.label} · ${endedAgo(turn.ended_at ?? turn.started_at, now)}`}
                  subtitle={`${capitalize(turn.advice)}${turn.shape_label ? ` (${turn.shape_label})` : ""}`}
                  showSeparator={index < stopped.length - 1}
                />
              ))
            )}
          </>
        )}
      </ScrollView>
    </Sheet>
  );
}

function capitalize(text: string): string {
  const t = text.trim();
  if (t === "") return t;
  return t.charAt(0).toUpperCase() + t.slice(1) + (/[.!?]$/.test(t) ? "" : ".");
}

const styles = StyleSheet.create({
  content: { padding: space.space16, gap: space.space12 },
  block: { gap: space.space4 },
});
