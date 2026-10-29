import * as Clipboard from "expo-clipboard";
import React, { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { useTheme } from "../../theme/ThemeProvider";
import { contrastReport, serializeThemeDocument, type ThemeDocument, type ThemePalette } from "../../theme/themeDocument";
import { DEFAULT_THEME_ID } from "../../theme/themePresets";
import { darkTokens, lightTokens, radii, space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { SectionHeader } from "../ds/SectionHeader";
import { useToast } from "../ds/ToastHost";

function Swatches({ colors }: { colors: string[] }) {
  return (
    <View style={styles.swatches}>
      {colors.map((color, index) => <View key={`${color}-${index}`} style={[styles.swatch, { backgroundColor: color }]} />)}
    </View>
  );
}

export function ThemePicker() {
  const { tokens, scheme, themeId, themes, activeDocument, setThemeId, importTheme, removeTheme } = useTheme();
  const toast = useToast();
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);

  const builtIn = scheme === "light" ? lightTokens : darkTokens;
  const options: { id: string; name: string; colors: string[] }[] = [
    { id: DEFAULT_THEME_ID, name: "Forge", colors: [builtIn.bg0, builtIn.bg2, builtIn.ink, builtIn.accent] },
    ...themes.map((doc) => ({ id: doc.id, name: doc.name, colors: [doc[scheme].bg0, doc[scheme].bg2, doc[scheme].ink, doc[scheme].accent] })),
  ];
  const report = contrastReport(activeDocument ? activeDocument[scheme] : (builtIn as unknown as ThemePalette));
  const warnings = report.filter((check) => !check.pass);

  const doImport = () => {
    try {
      const doc = importTheme(draft);
      setError(null);
      setDraft("");
      toast.show(`Imported ${doc.name}`);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Could not import this theme");
    }
  };

  const doExport = async (doc: ThemeDocument) => {
    const text = serializeThemeDocument(doc);
    setDraft(text);
    try {
      await Clipboard.setStringAsync(text);
      toast.show("Theme copied");
    } catch {
      toast.show("Theme is in the box below", { tone: "danger" });
    }
  };

  return (
    <View>
      <SectionHeader>Colour theme</SectionHeader>
      <View style={styles.grid} accessibilityRole="radiogroup">
        {options.map((option) => {
          const selected = option.id === themeId;
          return (
            <Pressable
              key={option.id}
              onPress={() => setThemeId(option.id)}
              accessibilityRole="radio"
              accessibilityState={{ checked: selected }}
              accessibilityLabel={`${option.name} theme`}
              style={[styles.card, { backgroundColor: tokens.bg2, borderColor: selected ? tokens.accent : tokens.border }]}
            >
              <Swatches colors={option.colors} />
              <Text style={[type.bodyBold, { color: tokens.ink }]} numberOfLines={1}>{option.name}</Text>
              {option.id.startsWith("custom:") ? <Text style={[type.sub, { color: tokens.ink3 }]}>Imported</Text> : null}
            </Pressable>
          );
        })}
      </View>

      <View style={styles.readout} accessibilityLabel="Contrast readout">
        <Text style={[type.bodyBold, { color: tokens.ink }]}>Contrast ({scheme})</Text>
        {report.map((check) => (
          <Text key={check.label} style={[type.sub, { color: check.pass ? tokens.ink3 : tokens.warn }]}>
            {`${check.pass ? "Pass" : "Low"} · ${check.label} ${check.ratio.toFixed(1)}:1 (needs ${check.min}:1)`}
          </Text>
        ))}
        {warnings.length > 0 ? <Text style={[type.sub, { color: tokens.warn }]}>{`${warnings.length} pair${warnings.length === 1 ? "" : "s"} below the WCAG minimum. Text may be hard to read.`}</Text> : null}
      </View>

      <View style={styles.importBox}>
        <Input
          label="Import or export a theme (JSON, up to 32 KiB)"
          value={draft}
          onChangeText={(text) => { setDraft(text); setError(null); }}
          multiline
          numberOfLines={5}
          mono
          autoCapitalize="none"
          autoCorrect={false}
          error={error ?? undefined}
          accessibilityLabel="Theme JSON"
        />
        <View style={styles.actions}>
          <Button label="Import" variant="secondary" onPress={doImport} disabled={!draft.trim()} />
          {activeDocument ? <Button label="Export current" variant="ghost" onPress={() => void doExport(activeDocument)} /> : null}
          {themeId.startsWith("custom:") ? <Button label="Remove" variant="danger" onPress={() => removeTheme(themeId)} /> : null}
        </View>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  grid: { flexDirection: "row", flexWrap: "wrap", gap: space.space8, paddingHorizontal: space.space16 },
  card: { width: 148, gap: space.space8, padding: space.space12, borderRadius: radii.radius4, borderWidth: 1.5 },
  swatches: { flexDirection: "row", gap: space.space4 },
  swatch: { width: 24, height: 24, borderRadius: radii.radius4, borderWidth: StyleSheet.hairlineWidth, borderColor: "rgba(128,128,128,0.4)" },
  readout: { gap: space.space4, paddingHorizontal: space.space16, paddingTop: space.space16 },
  importBox: { gap: space.space8, paddingHorizontal: space.space16, paddingTop: space.space16 },
  actions: { flexDirection: "row", flexWrap: "wrap", gap: space.space8 },
});
