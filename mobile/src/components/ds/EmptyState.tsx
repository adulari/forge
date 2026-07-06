// DESIGN_SYSTEM.md §6 Containers — EmptyState: 24px lucide icon (ink4), one sub
// sentence, optional secondary Button.
//
// The secondary action is a caller-supplied slot (`action`) rather than an
// internal `ds/Button` import: Button.tsx is owned by the parallel T1.1 task,
// and its exact prop signature isn't available while this file is written, so
// EmptyState stays decoupled from it. Callers render their own <Button
// variant="secondary" .../> (or any Pressable) into the slot.
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";

export interface EmptyStateIconProps {
  size?: number;
  color?: string;
  strokeWidth?: number;
}

/** Matches the lucide-react-native icon component shape without importing its internal types. */
export type EmptyStateIcon = React.ComponentType<EmptyStateIconProps>;

export interface EmptyStateProps {
  icon: EmptyStateIcon;
  message: string;
  /** Optional secondary action slot — render a ds Button or any Pressable. */
  action?: React.ReactNode;
}

export function EmptyState({ icon: Icon, message, action }: EmptyStateProps) {
  const tokens = useTokens();
  return (
    <View style={styles.container} accessibilityRole="text" accessibilityLabel={message}>
      <Icon size={24} color={tokens.ink4} strokeWidth={1.75} />
      <Text style={[type.sub, styles.message, { color: tokens.ink2 }]}>{message}</Text>
      {action ? <View style={styles.action}>{action}</View> : null}
    </View>
  );
}

const styles = StyleSheet.create({
  container: { alignItems: "center", justifyContent: "center", padding: space.space32, gap: space.space12 },
  message: { textAlign: "center" },
  action: { marginTop: space.space8 },
});
