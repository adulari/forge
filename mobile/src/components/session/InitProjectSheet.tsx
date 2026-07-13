import React from "react";
import { Text, View } from "react-native";

import type { RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Sheet } from "../ds/Sheet";

export function InitProjectSheet({ visible, onClose, send }: { visible: boolean; onClose: () => void; send: (input: RemoteInput) => boolean }) {
  const tokens = useTokens();
  const submit = () => { if (send({ kind: "prompt", text: "/init" })) onClose(); };
  return <Sheet visible={visible} onClose={onClose} accessibilityLabel="Initialize project guidance" snapPoints={[0.5]}><View style={{ paddingHorizontal: space.space16, paddingBottom: space.space16, gap: space.space12 }}><Text style={[type.heading, { color: tokens.ink }]}>Initialize project guidance</Text><Text style={[type.sub, { color: tokens.ink3 }]}>Forge will inspect this repository and create a concise <Text style={{ fontFamily: "monospace" }}>.forge/AGENTS.md</Text> with architecture, setup, testing, and convention guidance for future sessions.</Text><Button label="Inspect and create guidance" onPress={submit} fullWidth /></View></Sheet>;
}
