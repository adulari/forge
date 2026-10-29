// "Keep this computer awake": the host's `[power] keep_awake` setting, in Settings rather than
// buried in the configuration list, with the note that sleep is not the same as screen lock.
import React, { useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { useConfig, useUpdateConfig } from "../../lib/queries";
import {
  KEEP_AWAKE_LABELS,
  KEEP_AWAKE_LOCK_NOTE,
  KEEP_AWAKE_MODES,
  isKeepAwakeMode,
  keepAwakeExplanation,
  type KeepAwakeMode,
} from "../../lib/keepAwake";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Segmented } from "../ds/Segmented";
import { SectionHeader } from "../ds/SectionHeader";

export function HostPowerSection() {
  const tokens = useTokens();
  const query = useConfig();
  const update = useUpdateConfig();
  const [error, setError] = useState<string | null>(null);
  const field = query.data?.fields.find((candidate) => candidate.key === "power.keep_awake");
  if (!field || !isKeepAwakeMode(field.value)) return null;
  const mode: KeepAwakeMode = field.value;

  return (
    <View>
      <SectionHeader>This host</SectionHeader>
      <View style={styles.body}>
        <Text style={[type.body, { color: tokens.ink }]}>Keep this computer awake</Text>
        <Segmented
          testID="keep-awake"
          options={KEEP_AWAKE_MODES.map((value) => ({ value, label: KEEP_AWAKE_LABELS[value] }))}
          value={mode}
          onChange={(next) => {
            setError(null);
            update.mutate(
              { key: "power.keep_awake", value: next, scope: "user" },
              { onError: (reason) => setError(reason instanceof Error ? reason.message : "Could not save this setting") },
            );
          }}
        />
        <Text style={[type.sub, { color: error ? tokens.danger : tokens.ink2 }]} accessibilityLiveRegion="polite">
          {error ?? keepAwakeExplanation(mode)}
        </Text>
        <Text style={[type.sub, { color: tokens.ink3 }]}>{KEEP_AWAKE_LOCK_NOTE}</Text>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  body: { gap: space.space8, paddingHorizontal: space.space16, paddingVertical: space.space8 },
});
