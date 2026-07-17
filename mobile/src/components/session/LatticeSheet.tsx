// Hearth Lattice launcher — a command launcher over Forge's code-intelligence graph.
// The sheet takes a symbol and sends `/lattice <symbol>`; the definition card, callers,
// references and pending-diff impact chain render in the conversation/overlay (the wire
// carries none of that to the sheet). The explainer here previews what that view returns,
// including the untested-caller warning (illustrative, not live data).
import { Search } from "lucide-react-native";
import React, { useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import type { RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";

const RETURNS: string[] = [
  "Signature, file:line, and visibility of the definition",
  "Every reference and caller, jump-to across the repo",
  "Impact of your pending diff, flagging untested callers",
];

export function LatticeSheet({ visible, onClose, send }: { visible: boolean; onClose: () => void; send: (input: RemoteInput) => boolean }) {
  const tokens = useTokens();
  const { isCompact } = useBreakpoint();
  const [symbol, setSymbol] = useState("");
  const [failed, setFailed] = useState(false);

  const submit = () => {
    const value = symbol.trim();
    if (!value) return;
    if (send({ kind: "prompt", text: `/lattice ${value}` })) {
      setSymbol("");
      setFailed(false);
      onClose();
      return;
    }
    setFailed(true);
  };

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Inspect code symbol" snapPoints={[0.7]}>
      <View style={[styles.content, isCompact ? null : styles.contentWide]}>
        <View style={styles.titleRow}>
          <Text style={[type.headingBold, { color: tokens.ink }]}>Lattice</Text>
          <Text style={[styles.mono, { color: tokens.ink4 }]}>code graph</Text>
        </View>
        <Text style={[type.sub, styles.subtitle, { color: tokens.ink3 }]}>Trace a symbol&apos;s definition, callers, and blast radius through Forge&apos;s code-intelligence graph.</Text>

        <Text style={[type.section, styles.section, { color: tokens.ink4 }]}>symbol</Text>
        <Input
          value={symbol}
          onChangeText={setSymbol}
          placeholder="rank_candidates"
          mono
          autoCapitalize="none"
          autoCorrect={false}
          accessibilityLabel="Code symbol"
          returnKeyType="search"
          onSubmitEditing={submit}
          leading={<Search size={15} strokeWidth={1.75} color={tokens.ink3} />}
        />
        <Button label="Inspect symbol" onPress={submit} disabled={!symbol.trim()} fullWidth style={styles.inspectBtn} />

        <Text style={[type.section, styles.section, { color: tokens.ink4 }]}>what you&apos;ll see</Text>
        <View style={[styles.card, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}>
          <View style={[styles.gutter, { backgroundColor: tokens.accent }]} />
          <View style={styles.cardBody}>
            {RETURNS.map((line) => (
              <View key={line} style={styles.bulletRow}>
                <View style={[styles.dot, { backgroundColor: tokens.ink4 }]} />
                <Text style={[type.sub, styles.bulletText, { color: tokens.ink2 }]}>{line}</Text>
              </View>
            ))}
            <View style={[styles.chain, { borderTopColor: tokens.hairline }]}>
              <Text style={[type.meta, styles.chainLabel, { color: tokens.ink4 }]}>example impact chain</Text>
              <View style={styles.chainRow}>
                <Text style={[styles.mono, { color: tokens.accent, fontWeight: "700" }]}>rank_candidates</Text>
                <Text style={[styles.mono, { color: tokens.ink4 }]}>→</Text>
                <Text style={[styles.mono, { color: tokens.ink2 }]}>route_turn</Text>
                <Text style={[styles.mono, { color: tokens.ink4 }]}>→</Text>
                <Text style={[styles.mono, { color: tokens.warn }]}>relay_send ⚠ untested</Text>
              </View>
            </View>
          </View>
        </View>
        <Text style={[type.meta, styles.footnote, { color: tokens.ink4 }]}>Results open in the conversation. A stale index prompts a reindex first.</Text>

        {failed ? (
          <Text style={[type.sub, styles.error, { color: tokens.danger }]}>Not connected — reconnect to send commands.</Text>
        ) : null}
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space20, paddingBottom: space.space32 },
  contentWide: { paddingHorizontal: space.space32 },
  titleRow: { flexDirection: "row", alignItems: "baseline", gap: space.space8 },
  subtitle: { marginTop: 2 },
  section: { paddingTop: space.space20, paddingBottom: space.space8 },
  inspectBtn: { marginTop: space.space12 },
  card: { position: "relative", borderWidth: 1, borderRadius: radii.radius16, overflow: "hidden" },
  gutter: { position: "absolute", left: 0, top: 0, bottom: 0, width: 2 },
  cardBody: { padding: space.space16, gap: space.space8 },
  bulletRow: { flexDirection: "row", alignItems: "flex-start", gap: space.space8 },
  dot: { width: 4, height: 4, borderRadius: 2, marginTop: 7 },
  bulletText: { flex: 1 },
  chain: { marginTop: space.space4, paddingTop: space.space12, borderTopWidth: StyleSheet.hairlineWidth, gap: space.space8 },
  chainLabel: { textTransform: "uppercase", letterSpacing: 0.6 },
  chainRow: { flexDirection: "row", alignItems: "center", flexWrap: "wrap", gap: space.space8 },
  footnote: { marginTop: space.space12 },
  mono: { fontFamily: monoFamily.regular, fontSize: 11.5, lineHeight: 16 },
  error: { marginTop: space.space16 },
});
