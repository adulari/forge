import React, { useState } from "react";
import { Text, View } from "react-native";

import type { RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";

export function DuelSheet({ visible, onClose, send }: { visible: boolean; onClose: () => void; send: (input: RemoteInput) => boolean }) {
  const tokens = useTokens();
  const [task, setTask] = useState("");
  const submit = () => {
    const text = task.trim();
    if (text && send({ kind: "prompt", text: `/duel ${text}` })) {
      setTask("");
      onClose();
    }
  };
  return <Sheet visible={visible} onClose={onClose} accessibilityLabel="Start model duel" snapPoints={[0.55]}><View style={{ paddingHorizontal: space.space16, paddingBottom: space.space16, gap: space.space12 }}><Text style={[typeScale.heading, { color: tokens.ink }]}>Model duel</Text><Text style={[typeScale.sub, { color: tokens.ink3 }]}>Race Forge’s best models in isolated worktrees, then choose the winning implementation.</Text><Input label="Task" value={task} onChangeText={setTask} placeholder="Implement the rate limiter" multiline accessibilityLabel="Duel task" returnKeyType="send" onSubmitEditing={submit} /><Button label="Start duel" onPress={submit} disabled={!task.trim()} fullWidth /></View></Sheet>;
}
