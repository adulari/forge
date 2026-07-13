import React, { useState } from "react";
import { Text, View } from "react-native";

import type { RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";

export function LatticeSheet({ visible, onClose, send }: { visible: boolean; onClose: () => void; send: (input: RemoteInput) => boolean }) {
  const tokens = useTokens();
  const [symbol, setSymbol] = useState("");
  const submit = () => { const value = symbol.trim(); if (value && send({ kind: "prompt", text: `/lattice ${value}` })) { setSymbol(""); onClose(); } };
  return <Sheet visible={visible} onClose={onClose} accessibilityLabel="Inspect code symbol" snapPoints={[0.55]}><View style={{ paddingHorizontal: space.space16, paddingBottom: space.space16, gap: space.space12 }}><Text style={[type.heading, { color: tokens.ink }]}>Inspect code symbol</Text><Text style={[type.sub, { color: tokens.ink3 }]}>Trace definitions, callers, dependencies, and recent reasoning through Forge’s code-intelligence graph.</Text><Input label="Symbol" value={symbol} onChangeText={setSymbol} placeholder="SessionDriverHandle" autoCapitalize="none" autoCorrect={false} accessibilityLabel="Code symbol" returnKeyType="send" onSubmitEditing={submit} /><Button label="Inspect symbol" onPress={submit} disabled={!symbol.trim()} fullWidth /></View></Sheet>;
}
