// Create, edit or (for project crews) read a saved crew: name, description and 1-8 member rows.
import { Plus, Trash2 } from "lucide-react-native";
import React, { useEffect, useMemo, useState } from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";

import {
  CREW_EFFORTS,
  CREW_MAX_MEMBERS,
  CREW_TIERS,
  crewHasErrors,
  draftFromCrew,
  emptyCrewDraft,
  emptyMember,
  serializeCrew,
  validateCrew,
  type Crew,
  type CrewDraft,
  type CrewMember,
} from "../../lib/crews";
import { useDeleteCrew, useSaveCrew } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Chip } from "../ds/Chip";
import { ConfirmDialog } from "../ds/ConfirmDialog";
import { IconButton } from "../ds/IconButton";
import { Input } from "../ds/Input";
import { Segmented } from "../ds/Segmented";
import { Sheet } from "../ds/Sheet";
import { Switch } from "../ds/Switch";

const TIER_OPTIONS = [{ value: "", label: "Auto" }, ...CREW_TIERS.map((t) => ({ value: t, label: t }))];

function errorText(e: unknown, fallback: string): string {
  return e instanceof Error && e.message ? e.message : fallback;
}

export function CrewSheet({ visible, crew, onClose }: { visible: boolean; crew: Crew | null; onClose: () => void }) {
  const tokens = useTokens();
  const save = useSaveCrew();
  const remove = useDeleteCrew();
  const [draft, setDraft] = useState<CrewDraft>(emptyCrewDraft);
  const [tried, setTried] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const readOnly = crew != null && !crew.editable;
  const isNew = crew == null;

  useEffect(() => {
    if (!visible) return;
    setDraft(crew ? draftFromCrew(crew) : emptyCrewDraft());
    setTried(false);
    setConfirmDelete(false);
    save.reset();
    remove.reset();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible, crew]);

  const errors = useMemo(() => validateCrew(draft), [draft]);
  const shown = tried ? errors : { member: {} };

  const patchMember = (i: number, patch: Partial<CrewMember>) =>
    setDraft((d) => ({ ...d, members: d.members.map((m, j) => (j === i ? { ...m, ...patch } : m)) }));

  const submit = () => {
    setTried(true);
    if (crewHasErrors(errors)) return;
    save.mutate(serializeCrew(draft), { onSuccess: onClose });
  };

  const title = isNew ? "New crew" : readOnly ? crew.name : `Edit ${crew.name}`;
  const serverError = save.isError ? errorText(save.error, "Could not save crew") : remove.isError ? errorText(remove.error, "Could not delete crew") : null;

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel={title} snapPoints={[0.9]}>
      <ScrollView contentContainerStyle={styles.content} keyboardShouldPersistTaps="handled">
        <Text accessibilityRole="header" style={[type.heading, { color: tokens.ink }]}>
          {title}
        </Text>
        <Text style={[type.sub, { color: tokens.ink3 }]}>
          {readOnly
            ? "Project crew: defined in this project's .forge/crews folder, so it is read-only here."
            : "A crew is a named roster of subagents. Mention it as @crew:name in a prompt to have the model delegate to it."}
        </Text>

        <Input
          label="Name"
          value={draft.name}
          onChangeText={(name) => setDraft((d) => ({ ...d, name }))}
          autoCapitalize="none"
          autoCorrect={false}
          disabled={readOnly || !isNew}
          clearable={false}
          placeholder="review-team"
          error={shown.name}
          accessibilityLabel="Crew name"
          testID="crew-name"
        />
        <Input
          label="Description"
          value={draft.description}
          onChangeText={(description) => setDraft((d) => ({ ...d, description }))}
          disabled={readOnly}
          placeholder="What this crew is for"
          accessibilityLabel="Crew description"
          testID="crew-description"
        />

        <View style={styles.membersHeader}>
          <Text style={[type.section, { color: tokens.ink4 }]}>{`Members (${draft.members.length}/${CREW_MAX_MEMBERS})`}</Text>
        </View>
        {shown.members ? <Text style={[type.sub, { color: tokens.danger }]}>{shown.members}</Text> : null}

        {draft.members.map((m, i) => {
          const e = shown.member[i] ?? {};
          return (
            <View key={i} style={[styles.member, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}>
              <View style={styles.memberTop}>
                <Input
                  containerStyle={styles.flex}
                  label="Role"
                  value={m.role}
                  onChangeText={(role) => patchMember(i, { role })}
                  autoCapitalize="none"
                  autoCorrect={false}
                  clearable={false}
                  disabled={readOnly}
                  placeholder="reviewer"
                  error={e.role}
                  accessibilityLabel={`Member ${i + 1} role`}
                />
                {readOnly ? null : (
                  <View style={styles.trash}>
                  <IconButton
                    icon={<Trash2 size={18} strokeWidth={1.75} color={tokens.ink2} />}
                    onPress={() => setDraft((d) => ({ ...d, members: d.members.filter((_, j) => j !== i) }))}
                    accessibilityLabel={`Remove member ${i + 1}`}
                  />
                  </View>
                )}
              </View>

              <Text style={[type.meta, { color: tokens.ink3 }]}>Tier</Text>
              {readOnly ? (
                <Text style={[type.body, { color: tokens.ink }]}>{m.tier ?? "auto"}</Text>
              ) : (
                <Segmented options={TIER_OPTIONS} value={m.tier ?? ""} onChange={(v) => patchMember(i, { tier: v || null })} />
              )}

              <Text style={[type.meta, { color: tokens.ink3 }]}>Effort</Text>
              {readOnly ? (
                <Text style={[type.body, { color: tokens.ink }]}>{m.effort ?? "auto"}</Text>
              ) : (
                <View style={styles.chips}>
                  {[{ v: "", l: "auto" }, ...CREW_EFFORTS.map((x) => ({ v: x, l: x }))].map((o) => (
                    <Chip key={o.v} label={o.l} selected={(m.effort ?? "") === o.v} onPress={() => patchMember(i, { effort: o.v || null })} />
                  ))}
                </View>
              )}

              <View style={styles.switchRow}>
                <Text style={[type.body, { color: tokens.ink }]}>Read-only</Text>
                <Switch value={m.no_edits} onValueChange={(no_edits) => patchMember(i, { no_edits })} disabled={readOnly} accessibilityLabel={`Member ${i + 1} read-only`} />
              </View>

              <Input
                label="Brief"
                value={m.brief}
                onChangeText={(brief) => patchMember(i, { brief })}
                multiline
                numberOfLines={3}
                disabled={readOnly}
                clearable={false}
                placeholder="What this member should focus on"
                error={e.brief}
                accessibilityLabel={`Member ${i + 1} brief`}
              />
            </View>
          );
        })}

        {readOnly || draft.members.length >= CREW_MAX_MEMBERS ? null : (
          <Button
            label="Add member"
            variant="secondary"
            icon={<Plus size={16} strokeWidth={1.75} color={tokens.ink} />}
            onPress={() => setDraft((d) => ({ ...d, members: [...d.members, emptyMember()] }))}
            fullWidth
          />
        )}

        {serverError ? <Text style={[type.sub, { color: tokens.danger }]}>{serverError}</Text> : null}

        {readOnly ? (
          <Button label="Close" variant="secondary" onPress={onClose} fullWidth />
        ) : (
          <View style={styles.footer}>
            <Button label={isNew ? "Create crew" : "Save changes"} onPress={submit} loading={save.isPending} disabled={save.isPending} fullWidth />
            {isNew ? null : <Button label="Delete crew" variant="danger" onPress={() => setConfirmDelete(true)} fullWidth />}
          </View>
        )}
      </ScrollView>
      <ConfirmDialog
        visible={confirmDelete}
        title={`Delete ${crew?.name ?? "crew"}?`}
        message="The crew file is removed from your config. Prompts that mention it will no longer delegate."
        confirmLabel="Delete"
        destructive
        loading={remove.isPending}
        onCancel={() => setConfirmDelete(false)}
        onConfirm={() => {
          if (!crew) return;
          remove.mutate(crew.name, {
            onSuccess: () => {
              setConfirmDelete(false);
              onClose();
            },
            onError: () => setConfirmDelete(false),
          });
        }}
      />
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space16, paddingBottom: space.space32, gap: space.space12 },
  flex: { flex: 1 },
  membersHeader: { marginTop: space.space4 },
  member: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius12, padding: space.space12, gap: space.space8 },
  memberTop: { flexDirection: "row", alignItems: "flex-start", gap: space.space8 },
  trash: { marginTop: space.space16 },
  chips: { flexDirection: "row", flexWrap: "wrap", gap: space.space8 },
  switchRow: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", minHeight: 44 },
  footer: { gap: space.space8 },
});
