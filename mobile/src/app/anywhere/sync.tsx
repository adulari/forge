import * as Clipboard from "expo-clipboard";
import { router } from "expo-router";
import { ChevronDown, ChevronRight, Copy, Lock, RefreshCw } from "lucide-react-native";
import React, { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { SettingsShell } from "../(tabs)/settings";
import { BackLink } from "../../components/ds/BackLink";
import { Badge } from "../../components/ds/Badge";
import { Banner } from "../../components/ds/Banner";
import { Button } from "../../components/ds/Button";
import { Card } from "../../components/ds/Card";
import { Screen } from "../../components/ds/Screen";
import { SectionHeader } from "../../components/ds/SectionHeader";
import { Skeleton } from "../../components/ds/Skeleton";
import { Switch } from "../../components/ds/Switch";
import { useToast } from "../../components/ds/ToastHost";
import {
  ageLabel,
  categoryStatusLine,
  classifyAccountSyncError,
  syncHubMeta,
  syncPillModel,
  toggleLockReason,
} from "../../lib/accountSync";
import type { AccountSyncCategory } from "../../lib/api";
import { useAccountSync, useRunAccountSyncNow, useUpdateAccountSync } from "../../lib/queries";
import { useSyncToggleContext } from "../../lib/useSyncToggleContext";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";

const EXPLAINER =
  "Carry your setup from machine to machine: keys, skills, settings, MCP servers and instructions. " +
  "It is end-to-end encrypted, so the Forge Anywhere service cannot read any of it. " +
  "Nothing syncs until you turn it on, and each machine chooses for itself.";

export default function AnywhereSyncScreen() {
  const tokens = useTokens();
  const toast = useToast();
  const query = useAccountSync();
  const update = useUpdateAccountSync();
  const syncNow = useRunAccountSyncNow();
  const [pendingId, setPendingId] = useState<string | null>(null);
  const status = query.data;

  const context = useSyncToggleContext();

  const toggle = (category: AccountSyncCategory, enabled: boolean) => {
    setPendingId(category.id);
    update.mutate(
      { category: category.id, enabled },
      {
        onError: (error) => toast.show(classifyAccountSyncError(error).message, { tone: "danger" }),
        onSettled: () => setPendingId(null),
      },
    );
  };

  const pill = syncPillModel(status);
  const problem = query.isError ? classifyAccountSyncError(query.error) : null;

  return (
    <SettingsShell active="anywhere-sync">
      <Screen scroll contentContainerStyle={styles.content}>
        <View style={styles.column}>
          <BackLink label="Anywhere" onPress={() => router.replace("/anywhere")} />
          <Text style={[typeScale.title, { color: tokens.ink }]}>Sync across machines</Text>
          {pill ? (
            <View style={styles.statusLine} accessibilityLabel={pill.description}>
              <View style={[styles.statusDot, { backgroundColor: pill.tone === "success" ? tokens.success : pill.tone === "warn" ? tokens.warn : pill.tone === "danger" ? tokens.danger : tokens.ink4 }]} />
              <Text style={[typeScale.meta, { color: tokens.ink3 }]}>{syncHubMeta(status)}</Text>
            </View>
          ) : null}
          <Text style={[typeScale.sub, { color: tokens.ink2 }]}>{EXPLAINER}</Text>

          {query.isLoading ? <Skeleton height={220} /> : null}

          {problem ? (
            <Banner
              tone={problem.kind === "old-daemon" ? "neutral" : "danger"}
              message={problem.message}
              actionLabel={problem.kind === "old-daemon" ? undefined : "Retry"}
              onAction={problem.kind === "old-daemon" ? undefined : () => void query.refetch()}
            />
          ) : null}

          {status && !status.signed_in ? (
            <Banner
              tone="warn"
              message="This machine is not signed in to Forge Anywhere. Sign in first, then come back to choose what syncs."
              actionLabel="Open Forge Anywhere"
              onAction={() => router.push("/anywhere")}
            />
          ) : null}
          {status?.signed_in && !status.anywhere_enabled ? (
            <Banner tone="neutral" message="Forge Anywhere is switched off on this machine, so nothing syncs until it is on." />
          ) : null}
          {status?.last_error ? <Banner tone="warn" message={`Last sync problem: ${status.last_error}`} /> : null}

          {status ? (
            <>
              <View>
                <SectionHeader>WHAT SYNCS ON THIS MACHINE</SectionHeader>
                <Card padded={false}>
                  {status.categories.map((category, index) => (
                    <CategoryRow
                      key={category.id}
                      category={category}
                      last={index === status.categories.length - 1}
                      lockReason={toggleLockReason(category, context)}
                      disabled={!status.signed_in || pendingId === category.id}
                      onToggle={(enabled) => toggle(category, enabled)}
                    />
                  ))}
                </Card>
              </View>

              <View style={styles.syncNow}>
                <Button
                  label="Sync now"
                  variant="secondary"
                  icon={<RefreshCw size={18} color={tokens.ink} />}
                  loading={syncNow.isPending}
                  disabled={!status.signed_in || !status.categories.some((category) => category.enabled)}
                  onPress={() =>
                    syncNow.mutate(undefined, {
                      onError: (error) => toast.show(classifyAccountSyncError(error).message, { tone: "danger" }),
                    })
                  }
                />
                <Text style={[typeScale.meta, { color: tokens.ink3 }]}>Last sync {ageLabel(status.last_sync_at)}</Text>
              </View>

              {status.sign_ins.length > 0 ? (
                <View>
                  <SectionHeader>SIGN IN ON THIS MACHINE</SectionHeader>
                  <Text style={[typeScale.sub, styles.sectionNote, { color: tokens.ink2 }]}>
                    Logins never sync, only the fact that you use them. Run the command in a terminal on this machine.
                  </Text>
                  <Card padded={false}>
                    {status.sign_ins.map((signIn, index) => (
                      <View
                        key={signIn.provider}
                        style={[styles.row, index < status.sign_ins.length - 1 ? hairline(tokens.hairline) : null]}
                      >
                        <Text style={[typeScale.bodyBold, { color: tokens.ink }]}>{signIn.label}</Text>
                        <View style={styles.commandRow}>
                          <Text selectable style={[styles.command, { color: tokens.ink, backgroundColor: tokens.bg3 }]}>
                            {signIn.command}
                          </Text>
                          <Button
                            label="Copy"
                            variant="ghost"
                            icon={<Copy size={16} color={tokens.ink2} />}
                            accessibilityLabel={`Copy sign-in command for ${signIn.label}`}
                            onPress={() => {
                              void Clipboard.setStringAsync(signIn.command).then(() => toast.show("Command copied"));
                            }}
                          />
                        </View>
                      </View>
                    ))}
                  </Card>
                </View>
              ) : null}

              {status.missing_tools.length > 0 ? (
                <View>
                  <SectionHeader>MISSING TOOLS</SectionHeader>
                  <Text style={[typeScale.sub, styles.sectionNote, { color: tokens.ink2 }]}>
                    Other machines use these. Installing them here is optional.
                  </Text>
                  <Card padded={false}>
                    {status.missing_tools.map((tool, index) => (
                      <View
                        key={tool.tool}
                        style={[styles.row, index < status.missing_tools.length - 1 ? hairline(tokens.hairline) : null]}
                      >
                        <Text style={[typeScale.bodyBold, { color: tokens.ink }]}>{tool.label}</Text>
                        <Text selectable style={[styles.command, { color: tokens.ink2, backgroundColor: tokens.bg3 }]}>
                          {tool.hint}
                        </Text>
                      </View>
                    ))}
                  </Card>
                </View>
              ) : null}
            </>
          ) : null}
        </View>
      </Screen>
    </SettingsShell>
  );
}

function hairline(color: string) {
  return { borderBottomColor: color, borderBottomWidth: StyleSheet.hairlineWidth };
}

function CategoryRow({
  category,
  last,
  lockReason,
  disabled,
  onToggle,
}: {
  category: AccountSyncCategory;
  last: boolean;
  lockReason: string | null;
  disabled: boolean;
  onToggle: (enabled: boolean) => void;
}) {
  const tokens = useTokens();
  const [open, setOpen] = useState(false);
  const line = categoryStatusLine(category);
  const details = category.conflicts.length + category.skipped.length;
  const toneColor = line.tone === "warn" ? tokens.warn : tokens.ink3;

  return (
    <View style={!last ? hairline(tokens.hairline) : null}>
      <View style={styles.categoryTop}>
        <View style={styles.categoryCopy}>
          <Text style={[typeScale.bodyBold, { color: lockReason ? tokens.ink2 : tokens.ink }]}>{category.label}</Text>
          {lockReason ? (
            <View style={styles.lockLine}>
              <Lock size={12} strokeWidth={1.75} color={tokens.ink3} />
              <Text style={[typeScale.meta, { color: tokens.ink3 }]}>{lockReason}</Text>
            </View>
          ) : (
            <Text style={[typeScale.meta, { color: toneColor }]}>{line.text}</Text>
          )}
        </View>
        <Switch
          value={category.enabled}
          disabled={disabled || lockReason != null}
          onValueChange={onToggle}
          accessibilityLabel={`Sync ${category.label}`}
          testID={`sync-toggle-${category.id}`}
        />
      </View>
      {details > 0 ? (
        <Pressable
          onPress={() => setOpen(!open)}
          accessibilityRole="button"
          accessibilityState={{ expanded: open }}
          style={styles.detailsToggle}
        >
          {open ? <ChevronDown size={14} color={tokens.ink3} /> : <ChevronRight size={14} color={tokens.ink3} />}
          {category.conflicts.length > 0 ? <Badge label={`${category.conflicts.length} conflict${category.conflicts.length === 1 ? "" : "s"}`} tone="warn" /> : null}
          {category.skipped.length > 0 ? <Badge label={`${category.skipped.length} skipped`} tone="neutral" /> : null}
        </Pressable>
      ) : null}
      {open ? (
        <View style={styles.details}>
          {category.conflicts.map((conflict) => (
            <View key={`c-${conflict.item}`}>
              <Text style={[typeScale.sub, { color: tokens.ink }]}>{conflict.item}</Text>
              <Text style={[typeScale.meta, { color: tokens.warn }]}>{conflict.detail}</Text>
            </View>
          ))}
          {category.skipped.map((skipped) => (
            <View key={`s-${skipped.item}`}>
              <Text style={[typeScale.sub, { color: tokens.ink }]}>{skipped.item}</Text>
              <Text style={[typeScale.meta, { color: tokens.ink3 }]}>{skipped.reason}</Text>
            </View>
          ))}
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space24, paddingBottom: space.space32 },
  column: { gap: space.space16, width: "100%", maxWidth: 720 },
  statusLine: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  statusDot: { width: 6, height: 6, borderRadius: 3 },
  sectionNote: { paddingHorizontal: space.space4, paddingBottom: space.space8 },
  categoryTop: { flexDirection: "row", alignItems: "center", gap: space.space12, paddingHorizontal: space.space16, paddingVertical: space.space12 },
  categoryCopy: { flex: 1, gap: 2 },
  lockLine: { flexDirection: "row", alignItems: "center", gap: space.space4 },
  detailsToggle: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space16, paddingBottom: space.space12 },
  details: { gap: space.space8, paddingHorizontal: space.space16, paddingBottom: space.space12 },
  syncNow: { flexDirection: "row", alignItems: "center", gap: space.space12, flexWrap: "wrap" },
  row: { gap: space.space8, paddingHorizontal: space.space16, paddingVertical: space.space12 },
  commandRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  command: { flex: 1, fontFamily: monoFamily.regular, fontSize: 12, paddingHorizontal: space.space8, paddingVertical: space.space8, borderRadius: radii.radius8, overflow: "hidden" },
});
