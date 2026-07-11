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
import { AlertTriangle } from "lucide-react-native";
import React, { useEffect, useRef, useState } from "react";
import { StyleSheet, Text, useWindowDimensions, View } from "react-native";
import Animated, { FadeOut, useAnimatedStyle, useReducedMotion, withTiming } from "react-native-reanimated";

import { Badge } from "../ds/Badge";
import { Button } from "../ds/Button";
import { Card } from "../ds/Card";
import { CommitIcon } from "../ds/CommitIcon";
import { HeatEdge } from "../ds/HeatEdge";
import { useToast } from "../ds/ToastHost";
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
  send: (input: RemoteInput) => boolean;
}

// A large embedded diff must never push Allow/Deny off-screen (this card sits in a
// non-scrolling slot above the composer) — cap it at a fraction of the window height with its
// own internal scroll instead.
const DIFF_MAX_HEIGHT_RATIO = 0.4;

// How long Allow/Deny stay locked waiting for the real unlock (a new `prompt_seq`) before giving
// up and re-enabling them.
const ACK_TIMEOUT_MS = 5_000;

export function PermissionCard({ prompt, diff, promptSeq, send }: PermissionCardProps) {
  const tokens = useTokens();
  const reduced = useReducedMotion();
  const toast = useToast();
  const { height: windowHeight } = useWindowDimensions();
  const [lockedSeq, setLockedSeq] = useState<number | null>(null);
  const [committed, setCommitted] = useState<"allow" | "deny" | null>(null);
  const ackTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // A new snapshot with a different prompt_seq means the previous decision was
  // consumed (or this is a fresh prompt) — unlock and clear the commit icon.
  useEffect(() => {
    setLockedSeq(null);
    setCommitted(null);
    if (ackTimerRef.current != null) {
      clearTimeout(ackTimerRef.current);
      ackTimerRef.current = null;
    }
  }, [promptSeq]);

  useEffect(
    () => () => {
      if (ackTimerRef.current != null) clearTimeout(ackTimerRef.current);
    },
    [],
  );

  // DESIGN_SYSTEM.md §5.2 Approve/Deny commit — "the card's other actions fade to 0.4".
  const dim = useAnimatedStyle(() => ({
    opacity: withTiming(lockedSeq === promptSeq ? 0.4 : 1, {
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
    if (!send({ kind: "allow", yes, seq: promptSeq })) {
      setLockedSeq(null);
      setCommitted(null);
      toast.show("not sent — reconnect and try again", { tone: "danger" });
      haptics.mergeConflict();
      return;
    }

    // Safety net: only a new `prompt_seq` is supposed to unlock this card (see the effect
    // above), but that never comes if this tap's `send` didn't actually reach the daemon
    // (socket not open, dropped mid-flight, session died mid-turn) — Allow/Deny would stay
    // disabled forever with no way out. Give the real ack a few seconds, then unlock again so
    // the user can retry. The `prompt_seq` effect clears this timer the instant a real ack does
    // land, so it never fires against an already-resolved prompt.
    ackTimerRef.current = setTimeout(() => {
      ackTimerRef.current = null;
      setLockedSeq(null);
      setCommitted(null);
      toast.show("didn't confirm — check connection and try again", { tone: "danger" });
      haptics.mergeConflict();
    }, ACK_TIMEOUT_MS);
  };

  return (
    <Animated.View style={styles.wrap} exiting={reduced ? undefined : FadeOut.duration(durations.gentle)}>
      <HeatEdge state="waiting" />
      <Card variant="feature" style={[styles.card, { borderColor: tokens.danger }]}>
        <Animated.View style={dim}>
          <View style={styles.header}>
            <AlertTriangle size={16} strokeWidth={1.75} color={tokens.danger} />
            <Badge label="permission" tone="danger" />
          </View>

          <Text style={[typeScale.body, { color: tokens.ink }, styles.prompt]}>{prompt}</Text>

          {diff?.pending ? (
            <View style={styles.diffSlot}>
              <DiffCard diff={diff} maxHeight={windowHeight * DIFF_MAX_HEIGHT_RATIO} />
            </View>
          ) : null}

          <View style={styles.actions}>
            <Button
              label="Deny"
              variant="danger"
              onPress={() => respond(false)}
              disabled={locked}
              icon={committed === "deny" ? <CommitIcon kind="x" color={tokens.onAccent} /> : undefined}
              style={styles.actionBtn}
            />
            <Button
              label="Allow"
              variant="allow"
              onPress={() => respond(true)}
              disabled={locked}
              icon={committed === "allow" ? <CommitIcon kind="check" color={tokens.onAccent} /> : undefined}
              style={styles.actionBtn}
            />
          </View>
        </Animated.View>
      </Card>
    </Animated.View>
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
