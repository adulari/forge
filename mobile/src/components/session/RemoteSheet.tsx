import React, { useState } from "react";
import { Text, View } from "react-native";

import type { RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Segmented } from "../ds/Segmented";
import { Sheet } from "../ds/Sheet";

export function RemoteSheet({ visible, onClose, send }: { visible: boolean; onClose: () => void; send: (input: RemoteInput) => boolean }) {
  const tokens = useTokens();
  const [mode, setMode] = useState<"local" | "lan" | "anywhere">("local");
  const toggle = () => { if (send({ kind: "prompt", text: mode === "local" ? "/remote --local" : mode === "lan" ? "/remote --lan" : "/remote --anywhere" })) onClose(); };
  return <Sheet visible={visible} onClose={onClose} accessibilityLabel="Remote control exposure" snapPoints={[0.65]}><View style={{ paddingHorizontal: space.space16, paddingBottom: space.space16, gap: space.space12 }}><Text style={[type.heading, { color: tokens.ink }]}>Remote control</Text><Text style={[type.sub, { color: tokens.ink3 }]}>Choose how this session’s control surface is exposed. The same command toggles remote control off when it is already running.</Text><Segmented options={[{ value: "local", label: "Local" }, { value: "lan", label: "LAN" }, { value: "anywhere", label: "Anywhere" }]} value={mode} onChange={(value) => setMode(value as typeof mode)} /><Text style={[type.meta, { color: mode === "anywhere" ? tokens.warn : tokens.ink3 }]}>{mode === "local" ? "Only this device can connect." : mode === "lan" ? "Devices on your local network can connect." : "Creates a public tunnel. Share the link only with people you trust."}</Text><Button label="Toggle remote control" onPress={toggle} fullWidth /></View></Sheet>;
}
