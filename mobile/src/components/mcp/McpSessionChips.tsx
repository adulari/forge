import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { useMcp } from "../../lib/queries";
import { toggleToolDisabled } from "../../lib/mcpHelpers";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Chip } from "../ds/Chip";

/** One chip per enabled MCP server; unselecting a chip leaves that server out of the new session only. */
export function McpSessionChips({ off, onChange }: { off: string[]; onChange: (next: string[]) => void }) {
  const tokens = useTokens();
  const servers = (useMcp().data?.servers ?? []).filter((server) => server.enabled);
  if (servers.length === 0) return null;
  return (
    <View style={styles.wrap}>
      <Text style={[type.meta, { color: tokens.ink3 }]}>MCP servers for this session</Text>
      <View style={styles.row} accessibilityRole="list">
        {servers.map((server) => {
          const on = !off.includes(server.name);
          return (
            <Chip
              key={server.name}
              label={server.name}
              selected={on}
              size="compact"
              onPress={() => onChange(toggleToolDisabled(off, server.name, !on))}
              testID={`mcp-chip-${server.name}`}
            />
          );
        })}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { gap: space.space8 },
  row: { flexDirection: "row", flexWrap: "wrap", gap: space.space8 },
});
