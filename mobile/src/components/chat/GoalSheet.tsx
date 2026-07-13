import React, { useState } from "react";
import { Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";

export function GoalSheet({ visible, onClose, onSubmit }: { visible: boolean; onClose: () => void; onSubmit: (goal: string) => void }) {
  const tokens = useTokens();
  const [goal, setGoal] = useState("");
  const submit = () => { const value = goal.trim(); if (value) { onSubmit(`/goal ${value}`); setGoal(""); onClose(); } };
  return <Sheet visible={visible} onClose={onClose} accessibilityLabel="Start autonomous goal" snapPoints={[0.6]}><View style={{ paddingHorizontal: space.space16, paddingBottom: space.space16, gap: space.space12 }}><Text style={[type.heading, { color: tokens.ink }]}>Autonomous goal</Text><Text style={[type.sub, { color: tokens.ink3 }]}>Forge will make an ordered task plan, carry out the highest-value work, and continue until the goal is done.</Text><Input label="Goal" value={goal} onChangeText={setGoal} placeholder="Improve the app onboarding flow" multiline autoCapitalize="sentences" accessibilityLabel="Autonomous goal" returnKeyType="send" onSubmitEditing={submit} /><Button label="Set goal and start" onPress={submit} disabled={!goal.trim()} fullWidth /></View></Sheet>;
}
