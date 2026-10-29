import React from "react";
import { StyleSheet, Text, View } from "react-native";

import {
  setTerminalConfirmClose,
  setTerminalFontSize,
  setTerminalScrollSpeed,
  TERMINAL_FONT_SIZES,
  TERMINAL_SCROLL_SPEEDS,
  useTerminalPreferences,
} from "../../lib/terminalPreferences";
import { isWeb } from "../../lib/platform";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { ListRow } from "../ds/ListRow";
import { SectionHeader } from "../ds/SectionHeader";
import { Segmented } from "../ds/Segmented";
import { Switch } from "../ds/Switch";
import { useToast } from "../ds/ToastHost";

/** Terminal text size, wheel speed and the close prompt, kept on this device. */
export function TerminalSettingsSection() {
  const tokens = useTokens();
  const toast = useToast();
  const preferences = useTerminalPreferences();
  const save = (action: Promise<void>) => {
    void action.catch(() => toast.show("couldn't save terminal preference.", { tone: "danger" }));
  };

  return (
    <View>
      <SectionHeader>Terminal</SectionHeader>
      <View style={styles.group}>
        <Text style={[type.sub, styles.label, { color: tokens.ink3 }]}>Text size</Text>
        <Segmented
          options={TERMINAL_FONT_SIZES.map((size) => ({ value: String(size), label: `${size}` }))}
          value={String(preferences.fontSize)}
          onChange={(value) => save(setTerminalFontSize(Number(value)))}
        />
      </View>
      {isWeb ? (
        <View style={styles.group}>
          <Text style={[type.sub, styles.label, { color: tokens.ink3 }]}>Scroll speed</Text>
          <Segmented
            options={TERMINAL_SCROLL_SPEEDS.map((speed) => ({ value: String(speed), label: `${speed}×` }))}
            value={String(preferences.scrollSpeed)}
            onChange={(value) => save(setTerminalScrollSpeed(Number(value)))}
          />
        </View>
      ) : null}
      <ListRow
        title="Confirm before closing a running terminal"
        subtitle="Closing a terminal tab stops its shell. A shell that already exited closes without asking."
        showSeparator={false}
        trailing={
          <Switch
            value={preferences.confirmClose}
            onValueChange={(value) => save(setTerminalConfirmClose(value))}
            disabled={!preferences.loaded}
            accessibilityLabel="Confirm before closing a running terminal"
          />
        }
      />
      <Text style={[type.sub, styles.note, { color: tokens.ink3 }]}>
        Which shell a terminal starts, and which one the agent runs commands in, are set under [terminal] in the
        host&apos;s config.toml.
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  group: { gap: space.space8, paddingBottom: space.space12 },
  label: { paddingHorizontal: space.space16 },
  note: { paddingHorizontal: space.space16, paddingTop: space.space8 },
});
