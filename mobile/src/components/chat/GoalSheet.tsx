// Native Features pack — Goal launcher (companion to the "NF Goal" running banner).
// Hearth bottom sheet: dash-marked "Goal" label, a condition input, and a primary start
// action. Submitting sends `/goal <condition>`, which the CLI decomposes into a tracked
// task plan and then loops turns until the condition holds. The live run then surfaces
// through GoalBanner (components/session/GoalBanner.tsx). Contract unchanged so the
// Composer caller keeps working.
import { Target } from "lucide-react-native";
import React, { useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";

export function GoalSheet({ visible, onClose, onSubmit }: { visible: boolean; onClose: () => void; onSubmit: (goal: string) => void }) {
  const tokens = useTokens();
  const [goal, setGoal] = useState("");
  const submit = () => {
    const value = goal.trim();
    if (value) {
      onSubmit(`/goal ${value}`);
      setGoal("");
      onClose();
    }
  };
  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Start autonomous goal" snapPoints={[0.6]}>
      <View style={styles.content}>
        <View style={styles.label}>
          <View style={[styles.dash, { backgroundColor: tokens.accent }]} />
          <Text style={[type.section, { color: tokens.accent }]}>Goal</Text>
        </View>
        <View style={styles.title}>
          <Target size={18} strokeWidth={2} color={tokens.ink} />
          <Text style={[type.headingBold, { color: tokens.ink }]}>Loop until it&apos;s true</Text>
        </View>
        <Text style={[type.sub, { color: tokens.ink3 }]}>
          Forge decomposes the goal into an ordered task plan, works the highest-value step, then keeps re-running
          turns until the condition holds.
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
        <Button label="Set goal and start" onPress={submit} disabled={!goal.trim()} fullWidth />
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space16, paddingBottom: space.space16, gap: space.space12 },
  label: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  dash: { width: 6, height: 2 },
  title: { flexDirection: "row", alignItems: "center", gap: space.space8 },
});
