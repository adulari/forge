// Per-project management inside the picker sheet: rename, hide or show, and relocate a project
// whose folder moved. Controlled: the picker keeps the draft so a trip to the folder browser
// (for "Browse") does not lose what was typed.
import { ArrowLeft, Eye, EyeOff, Folder } from "lucide-react-native";
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { ApiError, type ProjectRow } from "../../lib/api";
import { manageChanges, type ProjectManageDraft } from "../../lib/projectOnboarding";
import { usePatchProjectEntry } from "../../lib/queriesProjects";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { ListRow } from "../ds/ListRow";

export interface ProjectManagePanelProps {
  project: ProjectRow;
  hidden: boolean;
  draft: ProjectManageDraft;
  onDraftChange: (draft: ProjectManageDraft) => void;
  onBack: () => void;
  onBrowseRelocate: () => void;
}

export function ProjectManagePanel({ project, hidden, draft, onDraftChange, onBack, onBrowseRelocate }: ProjectManagePanelProps) {
  const tokens = useTokens();
  const patch = usePatchProjectEntry();
  const changes = manageChanges(project, draft);
  const error = patch.error instanceof ApiError ? patch.error.message : patch.isError ? "Could not update this project" : null;

  const save = () => {
    if (changes) patch.mutate(changes, { onSuccess: onBack });
  };
  const toggleHidden = () => patch.mutate({ path: project.path, disabled: !hidden }, { onSuccess: onBack });

  return (
    <View style={styles.wrap}>
      <ListRow
        title="Back to projects"
        leading={<ArrowLeft size={16} strokeWidth={1.75} color={tokens.ink2} />}
        onPress={onBack}
        showSeparator={false}
      />
      <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Manage project</Text>
      <Text style={[typeScale.codeSmall, { color: tokens.ink3 }]} numberOfLines={2}>
        {project.path}
      </Text>
      <Input
        label="Name"
        value={draft.name}
        onChangeText={(name) => onDraftChange({ ...draft, name })}
        placeholder="Display name"
        accessibilityLabel="Project name"
      />
      <View style={styles.relocateRow}>
        <Input
          label="Moved to"
          value={draft.relocate}
          onChangeText={(relocate) => onDraftChange({ ...draft, relocate })}
          placeholder="/new/path"
          mono
          autoCapitalize="none"
          autoCorrect={false}
          containerStyle={styles.relocateInput}
          accessibilityLabel="New location"
        />
        <Button
          label="Browse"
          variant="secondary"
          icon={<Folder size={15} strokeWidth={1.8} color={tokens.ink2} />}
          onPress={onBrowseRelocate}
        />
      </View>
      <Button label="Save changes" onPress={save} disabled={!changes} loading={patch.isPending} fullWidth />
      <ListRow
        title={hidden ? "Show this project again" : "Hide this project"}
        subtitle={hidden ? "Brings it back to the recent list" : "Removes it from the list. Its sessions are untouched."}
        leading={hidden ? <Eye size={18} strokeWidth={1.75} color={tokens.ink2} /> : <EyeOff size={18} strokeWidth={1.75} color={tokens.ink2} />}
        onPress={toggleHidden}
        disabled={patch.isPending}
        showSeparator={false}
      />
      {error ? (
        <Text accessibilityRole="alert" style={[typeScale.sub, { color: tokens.danger }]}>
          {error}
        </Text>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { paddingHorizontal: space.space16, paddingBottom: space.space32, gap: space.space12 },
  relocateRow: { flexDirection: "row", alignItems: "flex-end", gap: space.space8 },
  relocateInput: { flex: 1 },
});
