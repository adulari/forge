// Which alerts Forge raises, where, and how loudly: the three categories, the in-app filter, the
// sound and "Hide notification text". The routing rules live in `lib/notifyPrefsCore.ts`.
import React, { useCallback } from "react";
import { Linking, Platform, StyleSheet, View } from "react-native";

import { useAuth } from "../../lib/auth";
import { setNotifyPrefs, useNotifyPrefs } from "../../lib/notifyPrefs";
import type { AlertFilter, NotifyPrefs } from "../../lib/notifyPrefsCore";
import { applyHideNotificationText } from "../../lib/push";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { ListRow } from "../ds/ListRow";
import { Segmented } from "../ds/Segmented";
import { SectionHeader } from "../ds/SectionHeader";
import { Switch } from "../ds/Switch";
import { useToast } from "../ds/ToastHost";

const FILTER_OPTIONS: { value: AlertFilter; label: string }[] = [
  { value: "all", label: "Always" },
  { value: "unfocused", label: "In background" },
];

export function NotificationPrefsSection() {
  const tokens = useTokens();
  const toast = useToast();
  const { baseUrl } = useAuth();
  const prefs = useNotifyPrefs();
  const android = Platform.OS === "android";

  const change = useCallback(
    (patch: Partial<NotifyPrefs>) => {
      void setNotifyPrefs(patch).catch(() => toast.show("couldn't save the notification setting.", { tone: "warn" }));
    },
    [toast],
  );

  const onHideText = useCallback(
    (hide: boolean) => {
      void setNotifyPrefs({ hideText: hide })
        .then(() => applyHideNotificationText(baseUrl, hide))
        .catch(() => toast.show("couldn't update the notification text setting.", { tone: "warn" }));
    },
    [baseUrl, toast],
  );

  const disabled = !prefs.loaded;
  return (
    <View>
      <SectionHeader>Alerts</SectionHeader>
      <ListRow
        title="Session finished"
        subtitle="A turn ended and the session is idle."
        trailing={<Switch value={prefs.done} onValueChange={(done) => change({ done })} disabled={disabled} accessibilityLabel="Alert when a session finishes" />}
      />
      <ListRow
        title="Needs attention"
        subtitle="Waiting on a permission or a question."
        trailing={
          <Switch value={prefs.attention} onValueChange={(attention) => change({ attention })} disabled={disabled} accessibilityLabel="Alert when a session needs attention" />
        }
      />
      <ListRow
        title="Failed"
        subtitle="A turn ended in an error."
        trailing={<Switch value={prefs.error} onValueChange={(error) => change({ error })} disabled={disabled} accessibilityLabel="Alert when a turn fails" />}
      />
      <View style={[styles.filter, { borderColor: tokens.border }]}>
        <ListRow
          title="Show alerts"
          subtitle={
            prefs.filter === "all"
              ? "Toast when open, system alert when not."
              : "System alerts, only when in the background."
          }
          showSeparator={false}
        />
        <View style={styles.segment}>
          <Segmented options={FILTER_OPTIONS} value={prefs.filter} onChange={(filter) => change({ filter })} />
        </View>
      </View>
      {android ? (
        <ListRow
          title="Sound and channels"
          subtitle="Set per channel in system settings."
          onPress={() => void Linking.openSettings()}
        />
      ) : (
        <ListRow
          title="Sound"
          subtitle="Play a short chime with each alert."
          trailing={<Switch value={prefs.sound} onValueChange={(sound) => change({ sound })} disabled={disabled} accessibilityLabel="Alert sound" />}
        />
      )}
      <ListRow
        title="Hide notification text"
        subtitle="Say only that a session needs you."
        showSeparator={false}
        trailing={<Switch value={prefs.hideText} onValueChange={onHideText} disabled={disabled} accessibilityLabel="Hide notification text" />}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  filter: {},
  segment: { paddingHorizontal: space.space16, paddingBottom: space.space12 },
});
