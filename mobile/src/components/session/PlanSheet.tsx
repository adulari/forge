// Native Features pack — Plan launcher. Hearth bottom sheet: dash-marked "Plan" label,
// an objective input, and a primary action. Submitting sends `/plan <objective>`; Forge
// investigates read-only and then presents a decision card for approval. The presented
// plan + its per-step state also render on the Plans screen (app/(tabs)/plans.tsx) and the
// in-session review card (components/review/PlanCard.tsx).
//
// `plan` is OPTIONAL and additive — the existing session-shell call site keeps working
// untouched. When passed (`Snapshot.plan`), the sheet shows the live plan's steps with their
// real v9 `status` above the objective field, so re-planning starts from what is actually done
// rather than from a blank box. Steps render through PlanCard's exported `PlanStepRow`, so the
// ✓/pulsing-dot/○ vocabulary is defined in exactly one place; a pre-v9 host (no `status`) shows
// plain ordinals there, same as the card.
// HANDOFF: the session shell (`app/session/[id]/_layout.tsx`) does not pass `plan` yet — wire
// `snapshot?.plan` through to light this up.
import { ClipboardList } from "lucide-react-native";
import React, { useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import type { Plan, RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";
import { PlanStepRow } from "../review/PlanCard";

export function PlanSheet({ visible, onClose, send, plan }: { visible: boolean; onClose: () => void; send: (input: RemoteInput) => boolean; plan?: Plan | null }) {
  const tokens = useTokens();
  const [task, setTask] = useState("");
  const submit = () => {
    const text = task.trim();
    if (text && send({ kind: "prompt", text: `/plan ${text}` })) {
      setTask("");
      onClose();
    }
  };
  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Create plan" snapPoints={[0.55]}>
      <View style={styles.content}>
        <View style={styles.label}>
          <View style={[styles.dash, { backgroundColor: tokens.accent }]} />
          <Text style={[typeScale.section, { color: tokens.accent }]}>Plan</Text>
        </View>
        <View style={styles.title}>
          <ClipboardList size={18} strokeWidth={2} color={tokens.ink} />
          <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Investigate, then plan</Text>
        </View>
        <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
          Forge explores the code in read-only mode, then presents a numbered implementation plan for you to approve,
          revise, or cancel.
        </Text>
        {plan && plan.steps.length > 0 ? (
          <View style={styles.steps}>
            <Text style={[typeScale.section, { color: tokens.ink4 }]}>current plan</Text>
            {plan.steps.map((step, index) => (
              <PlanStepRow key={index} step={step} index={index} />
            ))}
          </View>
        ) : null}
        <Input
          label="Objective"
          value={task}
          onChangeText={setTask}
          placeholder="Harden the APNs relay client"
          multiline
          autoCapitalize="sentences"
          accessibilityLabel="Plan objective"
          returnKeyType="send"
          onSubmitEditing={submit}
        />
        <Button label="Investigate and plan" onPress={submit} disabled={!task.trim()} fullWidth />
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space16, paddingBottom: space.space16, gap: space.space12 },
  label: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  dash: { width: 6, height: 2 },
  title: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  steps: { gap: space.space12 },
});
