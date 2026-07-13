import React, { useState } from "react";
import { Text, View } from "react-native";

import type { RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";

export function MemorySheet({ visible, onClose, send }: { visible: boolean; onClose: () => void; send: (input: RemoteInput) => boolean }) {
  const tokens = useTokens();
  const [memory, setMemory] = useState("");
  const remember = () => { const value = memory.trim(); if (value && send({ kind: "prompt", text: `/remember ${value}` })) { setMemory(""); onClose(); } };
  const browse = () => { if (send({ kind: "prompt", text: "/memories" })) onClose(); };
  return <Sheet visible={visible} onClose={onClose} accessibilityLabel="Project memories" snapPoints={[0.6]}><View style={{ paddingHorizontal: space.space16, paddingBottom: space.space16, gap: space.space12 }}><Text style={[type.heading, { color: tokens.ink }]}>Project memory</Text><Text style={[type.sub, { color: tokens.ink3 }]}>Save durable project facts so future Forge sessions start with the right context.</Text><Input label="Memory" value={memory} onChangeText={setMemory} placeholder="Uses pnpm and Vitest for verification" multiline accessibilityLabel="Project memory" returnKeyType="send" onSubmitEditing={remember} /><Button label="Remember" onPress={remember} disabled={!memory.trim()} fullWidth /><Button label="Browse project memories" variant="secondary" onPress={browse} fullWidth /></View></Sheet>;
}
