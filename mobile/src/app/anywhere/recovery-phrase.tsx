// Forge Anywhere — recovery phrase, shown once after a new-account sign-in
// (mobile.dc.html "AW Recovery Phrase", lines 211-252).
import { router } from "expo-router";
import React, { useEffect, useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { Button } from "../../components/ds/Button";
import { Input } from "../../components/ds/Input";
import { Screen } from "../../components/ds/Screen";
import { useAnywhere } from "../../lib/anywhere/store";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, tabularNums, type } from "../../theme/typography";

// AnywhereClient has no phrase-generation method — the recovery phrase is minted and
// held client-side only (never sent to the relay), and the mock backend doesn't model
// key material at all. This is a local, purely-visual placeholder grid (masked dots,
// matching the comp exactly) standing in for the 24-word BIP39-style phrase a real
// crypto layer would generate on this device.
const PLACEHOLDER_WORDS = Array.from({ length: 24 }, (_, i) => ({ n: i + 1, masked: "•••••••" }));

export default function AnywhereRecoveryPhraseScreen() {
  const tokens = useTokens();
  const { signedIn, loading } = useAnywhere();
  const [word7, setWord7] = useState("");
  const [word18, setWord18] = useState("");

  useEffect(() => {
    if (!loading && !signedIn) router.replace("/anywhere");
  }, [loading, signedIn]);

  const canContinue = word7.trim().length > 0 && word18.trim().length > 0;

  const columns = useMemo(() => {
    const left = PLACEHOLDER_WORDS.slice(0, 12);
    const right = PLACEHOLDER_WORDS.slice(12);
    return [left, right];
  }, []);

  if (loading || !signedIn) return null;

  return (
    <Screen scroll contentContainerStyle={styles.content}>
      <Text style={[type.headingBold, { color: tokens.ink }]}>Your recovery phrase</Text>
      <Text style={[type.sub, styles.warning, { color: tokens.ink2 }]}>
        Shown once. Write these 24 words down and keep them offline. Forge support cannot
        recover them.
      </Text>

      <View style={styles.grid}>
        {columns.map((col, ci) => (
          <View key={ci} style={styles.col}>
            {col.map((w) => (
              <View key={w.n} style={[styles.wordRow, { borderBottomColor: tokens.hairline }]}>
                <Text style={[styles.wordIndex, tabularNums, { color: tokens.ink4, fontFamily: monoFamily.regular }]}>
                  {w.n}
                </Text>
                <Text style={[styles.wordMask, { color: tokens.ink3, fontFamily: monoFamily.regular }]}>
                  {w.masked}
                </Text>
              </View>
            ))}
          </View>
        ))}
      </View>

      <View style={styles.confirmSection}>
        <Text style={[type.section, { color: tokens.ink4 }]}>confirm two words</Text>
        <View style={styles.confirmRow}>
          {/* The mock client has no real phrase to validate against — any non-empty entry
              is accepted here. Real checksum validation lands with the relay/crypto backend. */}
          <Input
            label="Word 7"
            value={word7}
            onChangeText={setWord7}
            mono
            autoCapitalize="none"
            autoCorrect={false}
            containerStyle={styles.confirmInput}
            accessibilityLabel="Word 7"
          />
          <Input
            label="Word 18"
            value={word18}
            onChangeText={setWord18}
            mono
            autoCapitalize="none"
            autoCorrect={false}
            containerStyle={styles.confirmInput}
            accessibilityLabel="Word 18"
          />
        </View>
      </View>

      <Button
        label="I wrote it down — continue"
        onPress={() => router.replace("/anywhere/first-host")}
        disabled={!canContinue}
        fullWidth
        style={styles.continueButton}
      />

      <View style={styles.secondaryRow}>
        <Pressable onPress={() => router.replace("/anywhere")} accessibilityRole="button" accessibilityLabel="Start over">
          <Text style={[type.bodyBold, { color: tokens.ink3 }]}>Start over</Text>
        </Pressable>
        <Pressable onPress={() => router.replace("/anywhere")} accessibilityRole="button" accessibilityLabel="Abandon setup">
          <Text style={[type.bodyBold, { color: tokens.ink3 }]}>Abandon setup</Text>
        </Pressable>
      </View>

      <Text style={[type.meta, styles.footer, { color: tokens.ink4 }]}>
        No cloud backup. No screenshots suggested. Held on paper, not in this app.
      </Text>
    </Screen>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space48 },
  warning: { marginTop: space.space4, lineHeight: 19 },
  grid: { flexDirection: "row", gap: space.space16, marginTop: space.space20 },
  col: { flex: 1 },
  wordRow: { flexDirection: "row", alignItems: "center", gap: 9, paddingVertical: 7, borderBottomWidth: StyleSheet.hairlineWidth },
  wordIndex: { width: 18, fontSize: 10.5, textAlign: "right" },
  wordMask: { fontSize: 12.5, letterSpacing: 1 },
  confirmSection: { marginTop: space.space24 },
  confirmRow: { flexDirection: "row", gap: space.space12, marginTop: space.space8 },
  confirmInput: { flex: 1 },
  continueButton: { marginTop: space.space32 },
  secondaryRow: { flexDirection: "row", justifyContent: "center", gap: space.space20, paddingVertical: space.space12 },
  footer: { textAlign: "center" },
});
