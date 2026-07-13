import React, { useState } from "react";
import { Text, View } from "react-native";

import type { RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";

export function CheckpointSheet({ visible, onClose, send }: { visible: boolean; onClose: () => void; send: (input: RemoteInput) => boolean }) {
  const tokens = useTokens();
  const [name, setName] = useState("");
  const save = () => { if (send({ kind: "prompt", text: name.trim() ? `/checkpoint ${name.trim()}` : "/checkpoint" })) { setName(""); onClose(); } };
  const restore = () => { if (send({ kind: "prompt", text: "/checkpoints" })) onClose(); };
  return <Sheet visible={visible} onClose={onClose} accessibilityLabel="Session checkpoints" snapPoints={[0.6]}><View style={{ paddingHorizontal: space.space16, paddingBottom: space.space16, gap: space.space12 }}><Text style={[type.heading, { color: tokens.ink }]}>Checkpoint</Text><Text style={[type.sub, { color: tokens.ink3 }]}>Save this conversation and workspace state before a risky change, or restore a prior checkpoint.</Text><Input label="Checkpoint name (optional)" value={name} onChangeText={setName} autoCapitalize="sentences" accessibilityLabel="Checkpoint name" returnKeyType="send" onSubmitEditing={save} /><Button label="Save checkpoint" onPress={save} fullWidth /><Button label="Browse and restore checkpoints" variant="secondary" onPress={restore} fullWidth /></View></Sheet>;
}
