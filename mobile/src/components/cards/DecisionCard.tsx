// Hearth core rule 2 ("One container only: the decision card") — the elevated preview for
// a session that needs a human: bg2/border/radius16 Card with a waiting HeatEdge, the live
// permission/question text (a short-lived socket attach, same pattern as FloorTile/
// DecisionPeek), and Respond (open the session)/Peek (answer inline via the existing
// DecisionPeek sheet) actions. Used by Fleet's needs-you rows and every Inbox row.
import { router } from "expo-router";
import React from "react";
import { StyleSheet, Text, View } from "react-native";
import Animated from "react-native-reanimated";

import type { SessionRow } from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { useSessionSocket } from "../../lib/ws";
import { useForgeline } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { formatCwd, monoFamily, type as typeScale } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { Button } from "../ds/Button";
import { Card } from "../ds/Card";
import { RelativeTime } from "../ds/RelativeTime";
import { StatusDot } from "../ds/StatusDot";

export interface DecisionCardProps {
  row: SessionRow;
  index: number;
  onPeek: (row: SessionRow) => void;
}

function DecisionCardBase({ row, index, onPeek }: DecisionCardProps) {
  const tokens = useTokens();
  const entrance = useForgeline(index);
  const { baseUrl } = useAuth();
  const { snapshot } = useSessionSocket(baseUrl, row.id);
  const title = row.title || `session ${row.id.slice(0, 8)}`;
  const hasWorktree = !!row.worktree;
  const preview = snapshot?.permission_prompt ?? snapshot?.question ?? null;

  return (
    <Animated.View style={[entrance, styles.wrap]}>
      <Card heatEdge="waiting">
        <View style={styles.headerRow}>
          <StatusDot state="waiting" />
          <Text style={[typeScale.headingBold, styles.title, { color: tokens.ink }]} numberOfLines={1}>
            {title}
          </Text>
          <RelativeTime timestampMs={row.last_activity * 1000} />
        </View>

        <Text style={[typeScale.body, styles.preview, { color: tokens.ink }]} numberOfLines={3}>
          {preview ?? "needs a decision"}
        </Text>

        <View style={styles.metaRow}>
          <Text
            style={[typeScale.codeSmall, styles.meta, { color: tokens.ink3, fontFamily: monoFamily.regular }]}
            numberOfLines={1}
          >
            {formatCwd(row.cwd)} · {row.model}
          </Text>
          {hasWorktree ? <Badge label="worktree" tone="outline" /> : null}
        </View>

        <View style={styles.actions}>
          <Button
            label="Respond"
            variant="primary"
            onPress={() => router.push(`/session/${row.id}`)}
            accessibilityLabel={`Respond to ${title}`}
            style={styles.respond}
          />
          <Button label="Peek" variant="secondary" onPress={() => onPeek(row)} accessibilityLabel={`Peek at ${title}`} />
        </View>
      </Card>
    </Animated.View>
  );
}

export const DecisionCard = React.memo(DecisionCardBase, (prev, next) => {
  const a = prev.row;
  const b = next.row;
  return (
    prev.index === next.index &&
    prev.onPeek === next.onPeek &&
    a.id === b.id &&
    a.title === b.title &&
    a.cwd === b.cwd &&
    a.worktree === b.worktree &&
    a.model === b.model &&
    a.last_activity === b.last_activity
  );
});

const styles = StyleSheet.create({
  wrap: { paddingHorizontal: space.space16, paddingTop: space.space4, paddingBottom: space.space12 },
  headerRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  title: { flex: 1 },
  preview: { marginTop: space.space8 },
  metaRow: { flexDirection: "row", alignItems: "center", gap: space.space8, marginTop: space.space8 },
  meta: { flex: 1 },
  actions: { flexDirection: "row", gap: space.space8, marginTop: space.space12 },
  respond: { flex: 1 },
});
