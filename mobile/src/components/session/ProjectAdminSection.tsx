// The picker's housekeeping rows: repair duplicate project entries and bring back hidden ones.
import { Eye, FolderX, Wrench } from "lucide-react-native";
import React, { useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { ApiError, type ProjectRow } from "../../lib/api";
import type { RepairResponse } from "../../lib/apiProjects";
import { repairSummary } from "../../lib/projectOnboarding";
import { usePatchProjectEntry, useRepairProjects } from "../../lib/queriesProjects";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { ListRow } from "../ds/ListRow";
import { SectionHeader } from "../ds/SectionHeader";

export function ProjectAdminSection({
  hidden,
  missing = [],
  onManageHidden,
  onManageMissing,
}: {
  hidden: readonly ProjectRow[];
  missing?: readonly ProjectRow[];
  onManageHidden: (project: ProjectRow) => void;
  onManageMissing?: (project: ProjectRow) => void;
}) {
  const tokens = useTokens();
  const repair = useRepairProjects();
  const patch = usePatchProjectEntry();
  const [result, setResult] = useState<RepairResponse | null>(null);
  const error = repair.error instanceof ApiError ? repair.error.message : repair.isError ? "Repair failed" : null;

  return (
    <View>
      <SectionHeader>Manage projects</SectionHeader>
      <ListRow
        title="Repair duplicates"
        subtitle="Merge entries that point at the same folder"
        leading={<Wrench size={18} strokeWidth={1.75} color={tokens.ink2} />}
        onPress={() => repair.mutate(undefined, { onSuccess: setResult })}
        disabled={repair.isPending}
        showSeparator={hidden.length + missing.length > 0}
      />
      {result ? (
        <View style={styles.result} accessibilityLiveRegion="polite">
          <Text style={[typeScale.sub, { color: tokens.ink2 }]}>{repairSummary(result)}</Text>
          {result.groups.map((group) => (
            <Text key={group.keep} style={[typeScale.codeSmall, { color: tokens.ink3 }]} numberOfLines={2}>
              {group.keep} absorbed {group.duplicates.length}: {group.duplicates.join(", ")}
            </Text>
          ))}
        </View>
      ) : null}
      {error ? (
        <Text accessibilityRole="alert" style={[typeScale.sub, styles.result, { color: tokens.danger }]}>
          {error}
        </Text>
      ) : null}
      {hidden.map((project, index) => (
        <ListRow
          key={project.path}
          title={project.name}
          subtitle={`Hidden · ${project.path}`}
          leading={<Eye size={18} strokeWidth={1.75} color={tokens.ink3} />}
          trailing={
            <Button
              label="Show"
              variant="secondary"
              onPress={() => patch.mutate({ path: project.path, disabled: false })}
              loading={patch.isPending && patch.variables?.path === project.path}
              accessibilityLabel={`Show ${project.name}`}
            />
          }
          hasInteractiveTrailing
          onPress={() => onManageHidden(project)}
          showSeparator={index < hidden.length - 1 || missing.length > 0}
          accessibilityLabel={`Manage hidden project ${project.name}`}
        />
      ))}
      {missing.map((project, index) => (
        <ListRow
          key={project.path}
          title={project.name}
          subtitle={`Folder not found · ${project.path}`}
          leading={<FolderX size={18} strokeWidth={1.75} color={tokens.danger} />}
          trailing={<Text style={[typeScale.sub, { color: tokens.ink3 }]}>Relocate</Text>}
          onPress={() => onManageMissing?.(project)}
          showSeparator={index < missing.length - 1}
          accessibilityLabel={`Relocate missing project ${project.name}`}
        />
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  result: { paddingHorizontal: space.space16, paddingVertical: space.space8, gap: space.space4 },
});
