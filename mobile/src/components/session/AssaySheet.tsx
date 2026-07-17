// Hearth Assay launcher — scope segmented control + what-it-does copy + start. Keeps the
// `/assay` / `/assay --diff` send behavior; the rich result view (AssayView) renders once the
// report lands in the transcript.
import { ShieldCheck } from "lucide-react-native";
import React, { useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import type { RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Segmented } from "../ds/Segmented";
import { Sheet } from "../ds/Sheet";

// Reviewer lenses the crew fans out across — mirrors the fan-out chips in the result view.
const LENSES = ["correctness", "safety", "coverage", "design", "architecture", "docs", "complexity"];

export function AssaySheet({ visible, onClose, send }: { visible: boolean; onClose: () => void; send: (input: RemoteInput) => boolean }) {
  const tokens = useTokens();
  const [scope, setScope] = useState<"repo" | "diff">("repo");

  const start = () => {
    if (send({ kind: "prompt", text: scope === "diff" ? "/assay --diff" : "/assay" })) onClose();
  };

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Run quality assay" snapPoints={[0.62]}>
      <View style={styles.content}>
        <View style={styles.titleRow}>
          <ShieldCheck size={20} strokeWidth={1.75} color={tokens.accent} />
          <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Quality assay</Text>
        </View>
        <Text style={[typeScale.sub, styles.subtitle, { color: tokens.ink3 }]}>
          A crew of critics reviews the code, then an adversarial pass re-derives each finding — only majority-verified
          findings survive.
        </Text>

        <Text style={[typeScale.section, styles.sectionLabel, { color: tokens.ink4 }]}>scope</Text>
        <Segmented
          options={[
            { value: "repo", label: "Repository" },
            { value: "diff", label: "Current diff" },
          ]}
          value={scope}
          onChange={(value) => setScope(value as typeof scope)}
        />
        <Text style={[typeScale.meta, styles.hint, { color: tokens.ink4 }]}>
          {scope === "diff" ? "Reviews only the working changes — faster, cheaper." : "Reviews all analyzable source in the tree."}
        </Text>

        <Text style={[typeScale.section, styles.sectionLabel, { color: tokens.ink4 }]}>lenses</Text>
        <View style={styles.lenses}>
          {LENSES.map((lens) => (
            <View key={lens} style={[styles.lensChip, { backgroundColor: tokens.bg3 }]}>
              <Text style={[typeScale.meta, { color: tokens.ink2 }]}>{lens}</Text>
            </View>
          ))}
        </View>

        <View style={styles.footer}>
          <Text style={[typeScale.meta, { color: tokens.ink4 }]}>You’ll choose analysis-only or permission-gated cleanup next.</Text>
          <Button label="Start assay" onPress={start} fullWidth />
        </View>
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space20, paddingBottom: space.space32 },
  titleRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  subtitle: { marginTop: space.space4 },
  sectionLabel: { paddingTop: space.space20, paddingBottom: space.space8 },
  hint: { marginTop: space.space8 },
  lenses: { flexDirection: "row", flexWrap: "wrap", gap: space.space8 },
  lensChip: { minHeight: 24, justifyContent: "center", paddingHorizontal: space.space8, borderRadius: radii.radiusPill },
  footer: { marginTop: space.space24, gap: space.space12 },
});
