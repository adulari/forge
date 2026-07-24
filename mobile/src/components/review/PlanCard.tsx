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
// matching decision. Controls remain disabled until the live question names this
// plan and contains explicit Build + Cancel options; option numbers are never guessed.
//
// HANDOFF(T3.3): ds/Button has no "danger-ghost" variant (only primary/secondary/
// ghost/danger/allow) — Cancel uses `variant="ghost"` today. Add a danger-ghost
// variant to ds/Button if the red tint from DESIGN_SYSTEM.md is required.
import React, { useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated, { FadeOut, useAnimatedStyle, useReducedMotion, withTiming } from "react-native-reanimated";

import { Button } from "../ds/Button";
import { Card } from "../ds/Card";
import { CommitIcon } from "../ds/CommitIcon";
import { IconButton } from "../ds/IconButton";
import { Input } from "../ds/Input";
import { useToast } from "../ds/ToastHost";
import { haptics } from "../../lib/haptics";
import { resolvePlanDecision } from "../../lib/planDecision";
import { type Plan, type PlanStep, type QuestionOption, type RemoteInput } from "../../lib/ws";
import { durations, easings, useEmberdot } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { tabularNums, type as typeScale } from "../../theme/typography";
import { Check, ChevronDown, ChevronRight, Send } from "lucide-react-native";

export interface PlanCardProps {
  plan: Plan;
  question: string | null;
  questionOptions: QuestionOption[];
  promptSeq: number;
  send: (input: RemoteInput) => boolean;
  onQueueAnswer?: (input: Extract<RemoteInput, { kind: "allow" | "answer" }>) => void;
}

export function PlanCard({ plan, question, questionOptions, promptSeq, send, onQueueAnswer }: PlanCardProps) {
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
  // Machined "Plan n/m" header + chevron collapse. Protocol v9 gave `PlanStep` a real `status`
  // (the status of the task an approved plan seeded from that step — see SnapPlanStep on the
  // wire), so the design comp's checkmark / pulsing-dot / hollow-circle step states are now
  // backed by data instead of fabricated. A pre-v9 host sends no `status` at all: the header
  // falls back to the plain step count and every step renders with its number, exactly as
  // before.
  const [collapsed, setCollapsed] = useState(false);

  useEffect(() => {
    setLockedSeq(null);
    setRevising(false);
    setReviseText("");
    setCommitted(null);
    setQueued(false);
  }, [promptSeq]);

  const tracked = plan.steps.some((step) => step.status != null);
  const doneCount = plan.steps.filter((step) => step.status === "done").length;
  const stepSummary = tracked
    ? `${doneCount}/${plan.steps.length} done`
    : `${plan.steps.length} step${plan.steps.length === 1 ? "" : "s"}`;

  const decision = resolvePlanDecision(plan.title, question, questionOptions, promptSeq);
  const locked = decision != null && lockedSeq === decision.promptSeq;

  // The card's other actions fade to 0.4 once a choice locks in.
  const dim = useAnimatedStyle(() => ({
    opacity: withTiming(locked ? 0.4 : 1, { duration: reduced ? 0 : durations.gentle, easing: easings.standard }),
  }));

  const commit = (text: string, haptic: () => void, which?: "approve" | "cancel") => {
    if (!decision || locked || text.trim().length === 0) return;
    setLockedSeq(decision.promptSeq);
    if (which) setCommitted(which);
    haptic();
    if (!send({ kind: "answer", text, seq: decision.promptSeq })) {
      if (onQueueAnswer) {
        onQueueAnswer({ kind: "answer", text, seq: decision.promptSeq });
        setQueued(true);
      } else {
        setLockedSeq(null);
        setCommitted(null);
        toast.show("not sent — reconnect and try again", { tone: "danger" });
      }
      haptics.mergeConflict();
    }
  };

  return (
    <Animated.View exiting={reduced ? undefined : FadeOut.duration(durations.gentle)}>
      <Card variant="feature" style={styles.card}>
        <Animated.View style={dim}>
          <Pressable
            onPress={() => setCollapsed((v) => !v)}
            accessibilityRole="button"
            accessibilityLabel={`${collapsed ? "expand" : "collapse"} plan`}
            accessibilityState={{ expanded: !collapsed }}
            style={styles.sectionLabel}
            hitSlop={8}
          >
            <View style={[styles.sectionDash, { backgroundColor: tokens.accent }]} />
            <Text style={[typeScale.section, { color: tokens.accent }]}>Plan</Text>
            <Text style={[typeScale.monoMeta, tabularNums, styles.stepCount, { color: tokens.ink3 }]}>
              {stepSummary}
            </Text>
            {collapsed ? (
              <ChevronRight size={14} strokeWidth={1.75} color={tokens.ink3} />
            ) : (
              <ChevronDown size={14} strokeWidth={1.75} color={tokens.ink3} />
            )}
          </Pressable>
          <Text style={[typeScale.heading, { color: tokens.ink }, styles.title]}>{plan.title}</Text>

          {!collapsed ? (
            <View style={styles.steps}>
              {plan.steps.map((step, idx) => (
                <PlanStepRow key={idx} step={step} index={idx} />
              ))}
            </View>
          ) : null}

          {plan.notes ? (
            <View style={[styles.notes, { backgroundColor: tokens.warnBg }]}>
              <Text style={[typeScale.sub, { color: tokens.warnBgInk }]}>{plan.notes}</Text>
            </View>
          ) : null}

          {queued ? <Text style={[typeScale.sub, { color: tokens.ink3 }]}>will send on reconnect</Text> : null}
          {!decision ? (
            <Text style={[typeScale.sub, { color: tokens.ink3 }]}>Waiting for approval request…</Text>
          ) : null}
          <View style={styles.actions}>
            <Button
              label="Approve"
              variant="allow"
              onPress={() => commit(decision?.build ?? "", haptics.allow, "approve")}
              disabled={locked || !decision}
              icon={committed === "approve" ? <CommitIcon kind="check" color={tokens.onAccent} /> : undefined}
              style={styles.approveBtn}
            />
            <Button
              label="Revise"
              variant="ghost"
              onPress={() => setRevising((v) => !v)}
              disabled={locked || !decision}
            />
            <Button
              label="Cancel"
              variant="ghost"
              onPress={() => commit(decision?.cancel ?? "", haptics.deny, "cancel")}
              disabled={locked || !decision}
              icon={committed === "cancel" ? <CommitIcon kind="x" color={tokens.ink2} /> : undefined}
            />
          </View>

          {revising ? (
            <View style={styles.reviseRow}>
              <Input
                value={reviseText}
                onChangeText={setReviseText}
                placeholder="what should change?"
                editable={!locked && decision != null}
                onSubmitEditing={() => commit(reviseText, haptics.select)}
                returnKeyType="send"
                containerStyle={styles.reviseInput}
                accessibilityLabel="plan revision"
              />
              <IconButton
                icon={<Send size={20} strokeWidth={1.75} color={tokens.ink} />}
                onPress={() => commit(reviseText, haptics.select)}
                disabled={!decision || locked || reviseText.trim().length === 0}
                accessibilityLabel="send revision"
              />
            </View>
          ) : null}
        </Animated.View>
      </Card>
    </Animated.View>
  );
}

/**
 * One numbered/marked plan step. The marker column carries the v9 `status`: ✓ for done (with an
 * ink3 strikethrough title), a pulsing accent dot for in_progress, a hollow ring for queued.
 * A step with no `status` (pre-v9 host) keeps its plain ordinal — the number is the honest
 * marker when there is no execution state to show.
 */
export function PlanStepRow({ step, index }: { step: PlanStep; index: number }) {
  const tokens = useTokens();
  const { dotStyle } = useEmberdot(step.status === "in_progress" ? "busy" : "idle");
  const done = step.status === "done";

  return (
    <View style={styles.step}>
      <View style={styles.stepMarker}>
        {step.status == null ? (
          <Text style={[typeScale.bodyBold, styles.stepNumber, { color: tokens.ink3 }]}>{index + 1}</Text>
        ) : done ? (
          <Check size={13} strokeWidth={2.5} color={tokens.ink3} />
        ) : step.status === "in_progress" ? (
          <Animated.View style={[styles.stepDot, { backgroundColor: tokens.accent }, dotStyle]} />
        ) : (
          <View style={[styles.stepRing, { borderColor: tokens.ink4 }]} />
        )}
      </View>
      <View style={styles.stepBody}>
        <Text
          style={[
            typeScale.bodyBold,
            done ? { color: tokens.ink3, textDecorationLine: "line-through" } : { color: tokens.ink },
          ]}
        >
          {step.title}
        </Text>
        {step.detail ? (
          <Text style={[typeScale.sub, { color: done ? tokens.ink3 : tokens.ink2 }, styles.stepDetail]}>{step.detail}</Text>
        ) : null}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  card: { gap: space.space8 },
  sectionLabel: { flexDirection: "row", alignItems: "center", gap: space.space8, minHeight: 24 },
  sectionDash: { width: 6, height: 2 },
  stepCount: { flex: 1 },
  title: { marginBottom: space.space4 },
  steps: { gap: space.space12 },
  step: { flexDirection: "row", gap: space.space12 },
  // Fixed marker column so number / check / dot / ring all sit on the same left edge, and the
  // step bodies stay aligned as statuses change mid-run.
  // `paddingTop` optically centers a glyph against the step title's 21px line box; the ordinal
  // is real text with its own line box, so it cancels that offset back out.
  stepMarker: { width: 20, alignItems: "center", justifyContent: "flex-start", paddingTop: 6 },
  stepNumber: { width: 20, marginTop: -6, textAlign: "right" },
  stepDot: { width: 6, height: 6, borderRadius: 3 },
  stepRing: { width: 9, height: 9, borderRadius: 4.5, borderWidth: 1.5 },
  stepBody: { flex: 1 },
  stepDetail: { marginTop: space.space2 },
  notes: { borderRadius: 8, padding: space.space12 },
  actions: { flexDirection: "row", gap: space.space8, marginTop: space.space4 },
  approveBtn: { flex: 1 },
  reviseRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  reviseInput: { flex: 1 },
});
