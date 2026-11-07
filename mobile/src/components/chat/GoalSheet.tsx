// Native Features pack — Goal launcher (companion to the "NF Goal" running banner).
// Hearth bottom sheet: dash-marked "Goal" label, a condition input, and a primary start
// action. Submitting sends `/goal <condition>`, which the CLI decomposes into a tracked
// task plan and then loops turns until the condition holds. The live run then surfaces
// through GoalBanner (components/session/GoalBanner.tsx). Contract unchanged so the
// Composer caller keeps working.
import { Target } from "lucide-react-native";
import React, { useEffect, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { goalStartCommand } from "../../lib/goalState";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";
import { Switch } from "../ds/Switch";

export function GoalSheet({
  visible,
  onClose,
  onSubmit,
  mode = "start",
  initialValue = "",
}: {
  visible: boolean;
  onClose: () => void;
  onSubmit: (goal: string) => void;
  /** `edit` rewords the goal that is running or paused instead of starting a new one. */
  mode?: "start" | "edit";
  initialValue?: string;
}) {
  const tokens = useTokens();
  const editing = mode === "edit";
  const [goal, setGoal] = useState(initialValue);
  const [contract, setContract] = useState(false);
  const [mission, setMission] = useState(false);
  useEffect(() => {
    if (visible) setGoal(initialValue);
  }, [visible, initialValue]);
  const submit = () => {
    const value = goal.trim();
    if (value) {
      onSubmit(editing ? `/goal edit ${value}` : goalStartCommand(value, contract, mission));
      setGoal("");
      onClose();
    }
  };
  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel={editing ? "Edit goal" : "Start autonomous goal"} snapPoints={[0.7]}>
      <View style={styles.content}>
        <View style={styles.label}>
          <View style={[styles.dash, { backgroundColor: tokens.accent }]} />
          <Text style={[type.section, { color: tokens.accent }]}>Goal</Text>
        </View>
        <View style={styles.title}>
          <Target size={18} strokeWidth={2} color={tokens.ink} />
          <Text style={[type.headingBold, { color: tokens.ink }]}>{editing ? "Reword the goal" : "Loop until it&apos;s true"}</Text>
        </View>
        <Text style={[type.sub, { color: tokens.ink3 }]}>
          {editing
            ? "The new wording applies from the next turn. Progress, budget and the task plan carry over."
            : "Forge decomposes the goal into an ordered task plan, works the highest-value step, then keeps re-running turns until the condition holds."}
        </Text>
        <Input
          label="Condition"
          value={goal}
          onChangeText={setGoal}
          placeholder="all mesh tests pass and failover picks by capability score"
          multiline
          autoCapitalize="sentences"
          accessibilityLabel="Goal condition"
          returnKeyType="send"
          onSubmitEditing={submit}
        />
        {editing ? null : (
          <View style={styles.contractRow}>
            <View style={styles.contractText}>
              <Text style={[type.bodyBold, { color: tokens.ink }]}>Validation contract</Text>
              <Text style={[type.sub, { color: tokens.ink3 }]}>
                Forge drafts 3 to 12 checks first and asks you to approve them. The goal is done only when an independent validator passes every one.
              </Text>
            </View>
            <Switch value={contract || mission} onValueChange={setContract} disabled={mission} accessibilityLabel="Validation contract" testID="goal-contract-switch" />
          </View>
        )}
        {editing ? null : (
          <View style={styles.contractRow}>
            <View style={styles.contractText}>
              <Text style={[type.bodyBold, { color: tokens.ink }]}>Mission</Text>
              <Text style={[type.sub, { color: tokens.ink3 }]}>
                For a big goal: Forge drafts 2 to 6 milestones, each with its own checks, and you approve them on one card. Each milestone runs in a fresh worker once the one before it passes.
              </Text>
            </View>
            <Switch value={mission} onValueChange={setMission} accessibilityLabel="Mission" testID="goal-mission-switch" />
          </View>
        )}
        <Button label={editing ? "Save wording" : "Set goal and start"} onPress={submit} disabled={!goal.trim()} fullWidth />
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space16, paddingBottom: space.space16, gap: space.space12 },
  label: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  dash: { width: 6, height: 2 },
  contractRow: { flexDirection: "row", alignItems: "center", gap: space.space12 },
  contractText: { flex: 1, gap: 2 },
  title: { flexDirection: "row", alignItems: "center", gap: space.space8 },
});
