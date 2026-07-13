import React, { useState } from "react";
import { Text, View } from "react-native";

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
  const submit = () => { const text = task.trim(); if (text && send({ kind: "prompt", text: `/plan ${text}` })) { setTask(""); onClose(); } };
  return <Sheet visible={visible} onClose={onClose} accessibilityLabel="Create plan" snapPoints={[0.55]}><View style={{ paddingHorizontal: space.space16, paddingBottom: space.space16, gap: space.space12 }}><Text style={[typeScale.heading, { color: tokens.ink }]}>Create plan</Text><Text style={[typeScale.sub, { color: tokens.ink3 }]}>Forge will investigate in read-only mode, then present an implementation plan for approval.</Text><Input label="Objective" value={task} onChangeText={setTask} placeholder="Add rate limiting" multiline accessibilityLabel="Plan objective" returnKeyType="send" onSubmitEditing={submit} /><Button label="Investigate and plan" onPress={submit} disabled={!task.trim()} fullWidth /></View></Sheet>;
}
