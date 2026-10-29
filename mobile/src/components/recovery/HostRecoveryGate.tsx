// Takes over the app when the connected daemon reports that its session store will not open
// (a newer schema, an interrupted migration). The daemon stays up in a recovery mode, so this
// screen can offer the ways out: restore a backup, run the migration again, open the data folder.
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { DatabaseBackup } from "lucide-react-native";
import React, { useCallback, useEffect, useState } from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";

import {
  ApiError,
  getRecovery,
  openRecoveryFolder,
  restoreRecoveryBackup,
  retryRecovery,
  type RecoveryOutcome,
} from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { clearStoreRecovery, useStoreRecovery } from "../../lib/hostStatus";
import { backupLabel } from "../../lib/recoveryCopy";
import { isTauri } from "../../lib/platform";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";
import { DESKTOP_WINDOW_CHROME_HEIGHT } from "../DesktopWindowChrome";
import { Button } from "../ds/Button";
import { ListRow } from "../ds/ListRow";
import { SectionHeader } from "../ds/SectionHeader";

export function HostRecoveryGate() {
  const recoveryHost = useStoreRecovery();
  const { baseUrl } = useAuth();
  if (!recoveryHost || recoveryHost !== baseUrl) return null;
  return <RecoveryScreen baseUrl={baseUrl} />;
}

function RecoveryScreen({ baseUrl }: { baseUrl: string }) {
  const tokens = useTokens();
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState<"retry" | "folder" | string | null>(null);
  const [outcome, setOutcome] = useState<RecoveryOutcome | null>(null);

  const status = useQuery({
    queryKey: ["host-recovery", baseUrl],
    queryFn: () => getRecovery(baseUrl),
    refetchInterval: 4_000,
    retry: false,
  });

  const finished = useCallback(() => {
    clearStoreRecovery();
    void queryClient.invalidateQueries();
  }, [queryClient]);

  // Once the daemon has moved on, its recovery routes are gone (404): the store opened.
  useEffect(() => {
    if (status.error instanceof ApiError && status.error.status === 404) finished();
  }, [status.error, finished]);

  const run = useCallback(
    async (key: string, action: () => Promise<RecoveryOutcome>) => {
      setBusy(key);
      setOutcome(null);
      try {
        const result = await action();
        setOutcome(result);
        if (result.ok) setTimeout(finished, 1_500);
        else void status.refetch();
      } catch (error) {
        setOutcome({ ok: false, message: error instanceof Error ? error.message : "That did not work." });
      } finally {
        setBusy(null);
      }
    },
    [finished, status],
  );

  const data = status.data;
  return (
    <View style={[styles.fill, { backgroundColor: tokens.bg0, top: isTauri ? DESKTOP_WINDOW_CHROME_HEIGHT : 0 }]} accessibilityViewIsModal testID="host-recovery">
      <ScrollView contentContainerStyle={styles.scroll}>
        <View style={styles.column}>
          <DatabaseBackup size={28} strokeWidth={1.75} color={tokens.warn} />
          <Text style={[type.title, { color: tokens.ink }]} accessibilityRole="header">
            Forge can’t open its session store
          </Text>
          <Text style={[type.body, { color: tokens.ink2 }]}>
            Your sessions are still on disk. The host stays up while you choose how to recover.
            Nothing is deleted: a store you replace is kept next to the new one.
          </Text>

          {data ? (
            <View style={[styles.errorBox, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}>
              <Text selectable style={[type.sub, { color: tokens.ink2, fontFamily: monoFamily.regular }]}>
                {data.error}
              </Text>
            </View>
          ) : status.isError && !(status.error instanceof ApiError && status.error.status === 404) ? (
            <Text style={[type.sub, { color: tokens.danger }]}>
              Could not reach the host’s recovery page. It may be restarting.
            </Text>
          ) : null}

          <View style={styles.actions}>
            <Button
              label="Run migration again"
              onPress={() => void run("retry", () => retryRecovery(baseUrl))}
              loading={busy === "retry"}
              disabled={busy != null || !data}
              testID="recovery-retry"
            />
            {isTauri && data?.can_open_folder ? (
              <Button
                label="Open data folder"
                variant="secondary"
                onPress={() => void run("folder", () => openRecoveryFolder(baseUrl))}
                loading={busy === "folder"}
                disabled={busy != null}
                testID="recovery-open-folder"
              />
            ) : null}
          </View>

          {outcome ? (
            <Text
              accessibilityLiveRegion="polite"
              style={[type.sub, { color: outcome.ok ? tokens.success : tokens.danger }]}
            >
              {outcome.message}
            </Text>
          ) : null}

          <View style={styles.backups}>
            <SectionHeader>Restore a backup</SectionHeader>
            {data && data.backups.length === 0 ? (
              <Text style={[type.sub, styles.empty, { color: tokens.ink3 }]}>
                No backups were found next to the store. `forge backup restore &lt;file&gt;` on the host
                can restore one from anywhere.
              </Text>
            ) : null}
            {(data?.backups ?? []).map((backup, index, all) => {
              const label = backupLabel(backup);
              return (
                <ListRow
                  key={backup.name}
                  title={label.title}
                  subtitle={label.subtitle}
                  showSeparator={index < all.length - 1}
                  hasInteractiveTrailing
                  trailing={
                    <Button
                      label="Restore"
                      variant="secondary"
                      onPress={() => void run(backup.name, () => restoreRecoveryBackup(baseUrl, backup.name))}
                      loading={busy === backup.name}
                      disabled={busy != null}
                      accessibilityLabel={`Restore ${label.title}`}
                    />
                  }
                />
              );
            })}
          </View>
        </View>
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { position: "absolute", top: 0, left: 0, right: 0, bottom: 0, zIndex: 1000 },
  scroll: { flexGrow: 1, alignItems: "center", paddingVertical: space.space32, paddingHorizontal: space.space16 },
  column: { width: "100%", maxWidth: 560, gap: space.space16 },
  errorBox: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4, padding: space.space12 },
  actions: { flexDirection: "row", flexWrap: "wrap", gap: space.space8 },
  // Rows and headers carry their own 16px inset; cancel the column's so they line up with the text above.
  backups: { marginHorizontal: -space.space16 },
  empty: { paddingHorizontal: space.space16, paddingVertical: space.space8 },
});
