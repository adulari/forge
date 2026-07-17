// Native Features pack — Plan launcher. Hearth bottom sheet: dash-marked "Plan" label,
// an objective input, and a primary action. Submitting sends `/plan <objective>`; Forge
// investigates read-only and then presents a decision card for approval. The presented
// plan + its per-step state render on the Plans screen (app/(tabs)/plans.tsx) and the
// in-session review card. Contract unchanged so the session-shell caller keeps working.
import { ClipboardList } from "lucide-react-native";
import React, { useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import type { RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";

export function PlanSheet({ visible, onClose, send }: { visible: boolean; onClose: () => void; send: (input: RemoteInput) => boolean }) {
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
});
