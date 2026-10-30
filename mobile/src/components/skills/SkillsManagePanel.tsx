import { FolderSync } from "lucide-react-native";
import React, { useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { Button } from "../ds/Button";
import { ConfirmDialog } from "../ds/ConfirmDialog";
import { useToast } from "../ds/ToastHost";
import { type SkillPack, type SkillScopeChoice } from "../../lib/skillsApi";
import { useRemoveSkillPack, useSkillsState, useSyncSkills } from "../../lib/skillsQueries";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";

export const PRECEDENCE_NOTE = "When a project skill and a user skill share a name, the project skill wins in that project.";

export function SkillsManagePanel({ session }: { session?: string | null }) {
  const tokens = useTokens();
  const toast = useToast();
  const state = useSkillsState(session);
  const sync = useSyncSkills();
  const remove = useRemoveSkillPack();
  const [confirm, setConfirm] = useState<SkillPack | null>(null);
  const packs = state.data?.packs ?? [];
  const targets = (state.data?.sync_targets ?? []).join(" and ");

  const runSync = (scope: SkillScopeChoice) =>
    sync.mutate(
      { scope, session },
      {
        onSuccess: (result) =>
          toast.show(
            result.conflicts.length > 0
              ? `Linked ${result.linked.length}, kept ${result.conflicts.length} existing folder${result.conflicts.length === 1 ? "" : "s"}`
              : `Linked ${result.linked.length} skill folder${result.linked.length === 1 ? "" : "s"}`,
            { tone: result.conflicts.length > 0 ? "warn" : "success" },
          ),
        onError: (error) => toast.show(error.message || "Sync failed", { tone: "danger" }),
      },
    );

  return (
    <View style={[styles.panel, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}>
      <Text style={[type.sub, { color: tokens.ink3 }]}>{PRECEDENCE_NOTE}</Text>
      <View style={styles.syncRow}>
        <View style={styles.syncText}>
          <Text style={[type.sub, { color: tokens.ink }]}>Share with Claude and Codex</Text>
          <Text style={[type.monoMeta, { color: tokens.ink4 }]}>{targets ? `Links skills into ${targets}.` : "Links skills where bridged agents look."}</Text>
        </View>
        <View style={styles.syncButtons}>
          <Button label="Sync mine" variant="secondary" icon={<FolderSync size={14} color={tokens.ink2} />} loading={sync.isPending} onPress={() => runSync("user")} accessibilityLabel="Sync my skills to .agents and .claude" />
          {session ? <Button label="Sync project" variant="secondary" loading={sync.isPending} onPress={() => runSync("project")} accessibilityLabel="Sync project skills to .agents and .claude" /> : null}
        </View>
      </View>
      {packs.length > 0 ? (
        <View style={styles.packs}>
          <Text style={[type.section, { color: tokens.ink4 }]}>Installed packs</Text>
          {packs.map((pack) => (
            <View key={`${pack.scope}:${pack.name}`} style={styles.packRow}>
              <View style={styles.syncText}>
                <Text style={[styles.packName, { color: tokens.ink }]} numberOfLines={1}>
                  {pack.name}
                  <Text style={{ color: tokens.ink4 }}>{`  ${pack.scope}`}</Text>
                </Text>
                <Text style={[type.monoMeta, { color: tokens.ink4 }]} numberOfLines={1}>
                  {pack.source}
                </Text>
              </View>
              <Button label="Remove" variant="ghost" onPress={() => setConfirm(pack)} accessibilityLabel={`Remove ${pack.name}`} />
            </View>
          ))}
        </View>
      ) : null}
      <ConfirmDialog
        visible={confirm != null}
        title={`Remove ${confirm?.name ?? "pack"}?`}
        message={`Deletes ${confirm?.skills.length ?? 0} installed skill folder${confirm?.skills.length === 1 ? "" : "s"}.`}
        confirmLabel="Remove"
        destructive
        loading={remove.isPending}
        onCancel={() => setConfirm(null)}
        onConfirm={() => {
          const pack = confirm;
          if (!pack) return;
          remove.mutate(
            { name: pack.name, scope: pack.scope, session },
            {
              onSuccess: () => toast.show(`Removed ${pack.name}`),
              onError: (error) => toast.show(error.message || "Could not remove", { tone: "danger" }),
              onSettled: () => setConfirm(null),
            },
          );
        }}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  panel: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius16, padding: space.space12, gap: space.space12 },
  syncRow: { gap: space.space8 },
  syncText: { flex: 1, minWidth: 0, gap: 2 },
  syncButtons: { flexDirection: "row", gap: space.space8, flexWrap: "wrap" },
  packs: { gap: space.space8 },
  packRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  packName: { fontSize: 13, fontFamily: monoFamily.bold },
});
