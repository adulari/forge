import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { Switch } from "../ds/Switch";
import { useConfirmedSyncToggle } from "./useConfirmedSyncToggle";
import { toggleLockReason } from "../../lib/accountSync";
import { useAccountSync, useUpdateAccountSync } from "../../lib/queries";
import { useSyncToggleContext } from "../../lib/useSyncToggleContext";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";

/** The per-category switches without the status detail, for onboarding. Renders nothing when the
 * daemon predates account sync or the machine is not signed in. */
export function AccountSyncToggles() {
  const tokens = useTokens();
  const query = useAccountSync();
  const update = useUpdateAccountSync();
  const context = useSyncToggleContext();
  const status = query.data;
  const { request, dialog } = useConfirmedSyncToggle((category, enabled) =>
    update.mutate({ category: category.id, enabled }),
  );
  if (!status || !status.signed_in) return null;
  return (
    <View style={styles.wrap}>
      <Text style={[type.heading, { color: tokens.ink }]}>Sync this setup across your machines</Text>
      <Text style={[type.sub, { color: tokens.ink2 }]}>
        End-to-end encrypted. Everything is off until you choose, and each machine chooses for itself.
      </Text>
      {status.categories.map((category) => {
        const locked = toggleLockReason(category, context);
        return (
          <View key={category.id} style={styles.row}>
            <Switch
              value={category.enabled}
              disabled={locked != null || update.isPending}
              onValueChange={(enabled) => request(category, enabled)}
              accessibilityLabel={`Sync ${category.label}`}
            />
            <Text style={[type.body, styles.label, { color: locked ? tokens.ink3 : tokens.ink }]}>
              {category.label}
              {locked ? ` (${locked.toLowerCase()})` : ""}
            </Text>
          </View>
        );
      })}
      {dialog}
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { gap: space.space8 },
  row: { flexDirection: "row", alignItems: "center", gap: space.space12 },
  label: { flex: 1 },
});
