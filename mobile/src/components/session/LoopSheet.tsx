import React, { useState } from "react";
import { Text, View } from "react-native";

import type { RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";

export function LoopSheet({ visible, onClose, send }: { visible: boolean; onClose: () => void; send: (input: RemoteInput) => boolean }) {
  const tokens = useTokens();
  const [task, setTask] = useState("");
  const submit = () => { const value = task.trim(); if (value && send({ kind: "prompt", text: `/loop ${value}` })) { setTask(""); onClose(); } };
  return <Sheet visible={visible} onClose={onClose} accessibilityLabel="Run autonomous loop" snapPoints={[0.6]}><View style={{ paddingHorizontal: space.space16, paddingBottom: space.space16, gap: space.space12 }}><Text style={[type.heading, { color: tokens.ink }]}>Autonomous loop</Text><Text style={[type.sub, { color: tokens.ink3 }]}>Forge will repeat turns on one task until it signals completion. Use for focused, well-bounded work.</Text><Input label="Task" value={task} onChangeText={setTask} placeholder="Fix the failing tests" multiline accessibilityLabel="Autonomous loop task" returnKeyType="send" onSubmitEditing={submit} /><Button label="Start loop" onPress={submit} disabled={!task.trim()} fullWidth /></View></Sheet>;
}
