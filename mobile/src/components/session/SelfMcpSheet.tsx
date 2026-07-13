import React, { useState } from "react";
import { Text, View } from "react-native";

import type { RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Sheet } from "../ds/Sheet";

export function SelfMcpSheet({ visible, onClose, send }: { visible: boolean; onClose: () => void; send: (input: RemoteInput) => boolean }) {
  const tokens = useTokens();
  const [status, setStatus] = useState<string | null>(null);
  const run = (command: string) => { if (send({ kind: "prompt", text: command })) { setStatus(command === "/self-mcp enable" ? "Enabled for future sessions." : command === "/self-mcp disable" ? "Disabled for future sessions." : "Status requested."); } };
  return <Sheet visible={visible} onClose={onClose} accessibilityLabel="Self MCP agent" snapPoints={[0.65]}><View style={{ paddingHorizontal: space.space16, paddingBottom: space.space16, gap: space.space12 }}><Text style={[type.heading, { color: tokens.ink }]}>Self MCP agent</Text><Text style={[type.sub, { color: tokens.ink3 }]}>Let Forge use a sub-Forge agent as tools for chat, quality assays, and interrupts. This setting applies to new sessions.</Text><Button label="Enable self-MCP" onPress={() => run("/self-mcp enable")} fullWidth /><Button label="Disable self-MCP" variant="secondary" onPress={() => run("/self-mcp disable")} fullWidth /><Button label="Check status" variant="secondary" onPress={() => run("/self-mcp status")} fullWidth />{status ? <Text style={[type.meta, { color: tokens.accent }]}>{status}</Text> : null}</View></Sheet>;
}
