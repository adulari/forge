// DESIGN_SYSTEM.md §6 PermissionCard: danger-edged feature Card; the prompt text
// body; DiffCard embedded when `diff.pending`; Allow/Deny bar.
// DESIGN_ELEVATION.md Move 1: a pending permission is "live" by construction
// (it only renders while `permission_prompt` is non-null) — it always carries
// the subtle HeatEdge accent, never a hard box.
//
// ARCHITECTURE.md §3 prompt_seq discipline: Allow/Deny echo the snapshot's
// `prompt_seq`. Buttons disable after the first tap and never retry — the next
// snapshot (a new `prompt_seq`) is what unlocks the card again, and by then the
// parent has usually stopped rendering this card entirely (permission_prompt
// went null).
import { AlertTriangle, Check, X } from "lucide-react-native";
import React, { useEffect, useState } from "react";
import { StyleSheet, Text, View } from "react-native";
import Animated, { useAnimatedStyle, useReducedMotion, withTiming } from "react-native-reanimated";

import { Badge } from "../ds/Badge";
import { Button } from "../ds/Button";
import { Card } from "../ds/Card";
import { HeatEdge } from "../ds/HeatEdge";
import { haptics } from "../../lib/haptics";
import { type Diff, type RemoteInput } from "../../lib/ws";
import { durations, easings } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { DiffCard } from "../review/DiffCard";

export interface PermissionCardProps {
  prompt: string;
  diff: Diff | null;
  promptSeq: number;
  send: (input: RemoteInput) => void;
}

export function PermissionCard({ prompt, diff, promptSeq, send }: PermissionCardProps) {
  const tokens = useTokens();
  const reduced = useReducedMotion();
  const [lockedSeq, setLockedSeq] = useState<number | null>(null);
  const [committed, setCommitted] = useState<"allow" | "deny" | null>(null);

  // A new snapshot with a different prompt_seq means the previous decision was
  // consumed (or this is a fresh prompt) — unlock and clear the commit icon.
  useEffect(() => {
    setLockedSeq(null);
    setCommitted(null);
  }, [promptSeq]);

  const dim = useAnimatedStyle(() => ({
    opacity: withTiming(lockedSeq === promptSeq ? 0.6 : 1, {
      duration: reduced ? 0 : durations.gentle,
      easing: easings.standard,
    }),
  }));

  const locked = lockedSeq === promptSeq;

  const respond = (yes: boolean) => {
    if (locked) return;
    setLockedSeq(promptSeq);
    setCommitted(yes ? "allow" : "deny");
    if (yes) haptics.allow();
    else haptics.deny();
    send({ kind: "allow", yes, seq: promptSeq });
  };

  return (
    <View style={styles.wrap}>
      <HeatEdge active />
      <Card variant="feature" style={[styles.card, { borderColor: tokens.danger }]}>
        <Animated.View style={dim}>
          <View style={styles.header}>
            <AlertTriangle size={16} strokeWidth={1.75} color={tokens.danger} />
            <Badge label="permission" tone="danger" />
          </View>

          <Text style={[typeScale.body, { color: tokens.ink }, styles.prompt]}>{prompt}</Text>

          {diff?.pending ? (
            <View style={styles.diffSlot}>
              <DiffCard diff={diff} />
            </View>
          ) : null}

          <View style={styles.actions}>
            <Button
              label="Deny"
              variant="danger"
              onPress={() => respond(false)}
              disabled={locked}
              fullWidth
              icon={committed === "deny" ? <X size={16} strokeWidth={2} color={tokens.onAccent} /> : undefined}
              style={styles.actionBtn}
            />
            <Button
              label="Allow"
              variant="allow"
              onPress={() => respond(true)}
              disabled={locked}
              fullWidth
              icon={committed === "allow" ? <Check size={16} strokeWidth={2} color={tokens.onAccent} /> : undefined}
              style={styles.actionBtn}
            />
          </View>
        </Animated.View>
      </Card>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { position: "relative" },
  card: { borderWidth: 1.5, marginLeft: space.space4 },
  header: { flexDirection: "row", alignItems: "center", gap: space.space8, marginBottom: space.space8 },
  prompt: { marginBottom: space.space12 },
  diffSlot: { marginBottom: space.space12 },
  actions: { flexDirection: "row", gap: space.space12 },
  actionBtn: { flex: 1 },
});
