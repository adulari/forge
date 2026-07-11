// DESIGN_SYSTEM.md §6 PlanCard: feature Card; "⬡ PLAN" section label ember;
// title heading; numbered steps (bodyBold + sub detail); warn notes block;
// action bar Approve(allow-variant) / Revise(ghost -> reveals free-text row) /
// Cancel(danger-ghost) via the plan's Build-it/Cancel option-number `answer`
// mechanic (FEATURES.md §1.2 "plan (+ pending 'Build it' question)").
//
// The pending plan's approval is carried on the SAME `question`/`question_options`
// /`prompt_seq` fields as any other question (ARCHITECTURE.md §3) — this card is
// handed that slice by its caller (review.tsx) rather than reading the session
// context itself, so it stays a plain, testable view over the plan + the
// matching decision. Approve/Cancel look up the option whose label reads
// "build"/"cancel" (case-insensitive) and answer with its 1-based index as a
// string; if no matching options are present (server hasn't attached them yet)
// they fall back to the conventional "1" (build)/"2" (cancel) ordering.
//
// HANDOFF(T3.3): ds/Button has no "danger-ghost" variant (only primary/secondary/
// ghost/danger/allow) — Cancel uses `variant="ghost"` today. Add a danger-ghost
// variant to ds/Button if the red tint from DESIGN_SYSTEM.md is required.
import React, { useEffect, useState } from "react";
import { StyleSheet, Text, View } from "react-native";
import Animated, { FadeOut, useAnimatedStyle, useReducedMotion, withTiming } from "react-native-reanimated";

import { Button } from "../ds/Button";
import { Card } from "../ds/Card";
import { CommitIcon } from "../ds/CommitIcon";
import { IconButton } from "../ds/IconButton";
import { Input } from "../ds/Input";
import { useToast } from "../ds/ToastHost";
import { haptics } from "../../lib/haptics";
import { type Plan, type QuestionOption, type RemoteInput } from "../../lib/ws";
import { durations, easings } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Send } from "lucide-react-native";

export interface PlanCardProps {
  plan: Plan;
  questionOptions: QuestionOption[];
  promptSeq: number;
  send: (input: RemoteInput) => boolean;
  onQueueAnswer?: (input: Extract<RemoteInput, { kind: "allow" | "answer" }>) => void;
}

function findOptionNumber(options: QuestionOption[], pattern: RegExp, fallback: string): string {
  const idx = options.findIndex((o) => pattern.test(o.label));
  return idx >= 0 ? String(idx + 1) : fallback;
}

export function PlanCard({ plan, questionOptions, promptSeq, send, onQueueAnswer }: PlanCardProps) {
  const tokens = useTokens();
  const toast = useToast();
  const reduced = useReducedMotion();
  const [lockedSeq, setLockedSeq] = useState<number | null>(null);
  const [revising, setRevising] = useState(false);
  const [reviseText, setReviseText] = useState("");
  // DESIGN_SYSTEM.md §5.2 Approve/Deny commit: which action was tapped, for the
  // check/x CommitIcon — only Approve/Cancel are binary commits; Revise opens a
  // free-text row instead of resolving the prompt, so it never sets this.
  const [committed, setCommitted] = useState<"approve" | "cancel" | null>(null);
  const [queued, setQueued] = useState(false);

  useEffect(() => {
    setLockedSeq(null);
    setRevising(false);
    setReviseText("");
    setCommitted(null);
    setQueued(false);
  }, [promptSeq]);

  const locked = lockedSeq === promptSeq;

  // The card's other actions fade to 0.4 once a choice locks in.
  const dim = useAnimatedStyle(() => ({
    opacity: withTiming(locked ? 0.4 : 1, { duration: reduced ? 0 : durations.gentle, easing: easings.standard }),
  }));

  const commit = (text: string, haptic: () => void, which?: "approve" | "cancel") => {
    if (locked || text.trim().length === 0) return;
    setLockedSeq(promptSeq);
    if (which) setCommitted(which);
    haptic();
    if (!send({ kind: "answer", text, seq: promptSeq })) {
      if (onQueueAnswer) {
        onQueueAnswer({ kind: "answer", text, seq: promptSeq });
        setQueued(true);
      } else {
        setLockedSeq(null);
        setCommitted(null);
        toast.show("not sent — reconnect and try again", { tone: "danger" });
      }
      haptics.mergeConflict();
    }
  };

  const approveNumber = findOptionNumber(questionOptions, /build/i, "1");
  const cancelNumber = findOptionNumber(questionOptions, /cancel/i, "2");

  return (
    <Animated.View exiting={reduced ? undefined : FadeOut.duration(durations.gentle)}>
      <Card variant="feature" style={styles.card}>
        <Animated.View style={dim}>
          <Text style={[typeScale.section, { color: tokens.accent }]}>⬡ PLAN</Text>
          <Text style={[typeScale.heading, { color: tokens.ink }, styles.title]}>{plan.title}</Text>

          <View style={styles.steps}>
            {plan.steps.map((step, idx) => (
              <View key={idx} style={styles.step}>
                <Text style={[typeScale.bodyBold, { color: tokens.ink3 }, styles.stepNumber]}>{idx + 1}</Text>
                <View style={styles.stepBody}>
                  <Text style={[typeScale.bodyBold, { color: tokens.ink }]}>{step.title}</Text>
                  {step.detail ? (
                    <Text style={[typeScale.sub, { color: tokens.ink2 }, styles.stepDetail]}>{step.detail}</Text>
                  ) : null}
                </View>
              </View>
            ))}
          </View>

          {plan.notes ? (
            <View style={[styles.notes, { backgroundColor: tokens.warnBg }]}>
              <Text style={[typeScale.sub, { color: tokens.warnBgInk }]}>{plan.notes}</Text>
            </View>
          ) : null}

          {queued ? <Text style={[typeScale.sub, { color: tokens.ink3 }]}>will send on reconnect</Text> : null}
          <View style={styles.actions}>
            <Button
              label="Cancel"
              variant="ghost"
              onPress={() => commit(cancelNumber, haptics.deny, "cancel")}
              disabled={locked}
              icon={committed === "cancel" ? <CommitIcon kind="x" color={tokens.ink2} /> : undefined}
            />
            <Button
              label="Revise"
              variant="ghost"
              onPress={() => setRevising((v) => !v)}
              disabled={locked}
            />
            <Button
              label="Approve"
              variant="allow"
              onPress={() => commit(approveNumber, haptics.allow, "approve")}
              disabled={locked}
              icon={committed === "approve" ? <CommitIcon kind="check" color={tokens.onAccent} /> : undefined}
              style={styles.approveBtn}
            />
          </View>

          {revising ? (
            <View style={styles.reviseRow}>
              <Input
                value={reviseText}
                onChangeText={setReviseText}
                placeholder="what should change?"
                editable={!locked}
                onSubmitEditing={() => commit(reviseText, haptics.select)}
                returnKeyType="send"
                containerStyle={styles.reviseInput}
                accessibilityLabel="plan revision"
              />
              <IconButton
                icon={<Send size={20} strokeWidth={1.75} color={tokens.ink} />}
                onPress={() => commit(reviseText, haptics.select)}
                disabled={locked || reviseText.trim().length === 0}
                accessibilityLabel="send revision"
              />
            </View>
          ) : null}
        </Animated.View>
      </Card>
    </Animated.View>
  );
}

const styles = StyleSheet.create({
  card: { gap: space.space8 },
  title: { marginBottom: space.space4 },
  steps: { gap: space.space12 },
  step: { flexDirection: "row", gap: space.space12 },
  stepNumber: { width: 20, textAlign: "right" },
  stepBody: { flex: 1 },
  stepDetail: { marginTop: space.space2 },
  notes: { borderRadius: 8, padding: space.space12 },
  actions: { flexDirection: "row", gap: space.space8, marginTop: space.space4 },
  approveBtn: { flex: 1 },
  reviseRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  reviseInput: { flex: 1 },
});
