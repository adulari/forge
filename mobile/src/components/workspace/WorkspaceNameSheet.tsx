// Name prompt for creating and renaming workspace entries. The daemon owns validity (it refuses
// `..`, `.git`, and anything that already exists), so its message is shown as-is under the field.
import React, { useEffect, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";

export interface WorkspaceNameSheetProps {
  visible: boolean;
  title: string;
  description?: string;
  label: string;
  initialValue: string;
  confirmLabel: string;
  busy: boolean;
  error: string | null;
  onSubmit: (value: string) => void;
  onClose: () => void;
}

export function WorkspaceNameSheet({
  visible,
  title,
  description,
  label,
  initialValue,
  confirmLabel,
  busy,
  error,
  onSubmit,
  onClose,
}: WorkspaceNameSheetProps) {
  const tokens = useTokens();
  const [value, setValue] = useState(initialValue);
  useEffect(() => {
    if (visible) setValue(initialValue);
  }, [visible, initialValue]);
  const submit = () => {
    if (!busy && value.trim()) onSubmit(value);
  };
  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel={title} snapPoints={[0.5]}>
      <View style={styles.body}>
        <Text style={[typeScale.heading, { color: tokens.ink }]}>{title}</Text>
        {description ? <Text style={[typeScale.sub, { color: tokens.ink3 }]}>{description}</Text> : null}
        <Input
          label={label}
          value={value}
          onChangeText={setValue}
          error={error ?? undefined}
          mono
          autoFocus
          autoCapitalize="none"
          autoCorrect={false}
          returnKeyType="done"
          onSubmitEditing={submit}
          accessibilityLabel={label}
        />
        <Button label={confirmLabel} onPress={submit} disabled={!value.trim()} loading={busy} fullWidth />
        <Button label="Cancel" variant="ghost" onPress={onClose} disabled={busy} fullWidth />
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  body: { paddingHorizontal: space.space16, paddingBottom: space.space16, gap: space.space12 },
});
