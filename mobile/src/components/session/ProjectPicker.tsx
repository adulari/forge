import { ApiError } from "../../lib/api";
import { canChooseNativeFolder, chooseNativeFolder } from "../../lib/folderPicker";
import { projectChoices, projectName } from "../../lib/projectSelection";
import { useBrowseProjects, useProjects } from "../../lib/queries";
import { useAuth } from "../../lib/auth";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Card } from "../ds/Card";
import { Input } from "../ds/Input";
import { ListRow } from "../ds/ListRow";
import { SectionHeader } from "../ds/SectionHeader";
import { Sheet } from "../ds/Sheet";
import { Skeleton } from "../ds/Skeleton";
import {
  ArrowLeft,
  ArrowUp,
  Check,
  ChevronRight,
  Folder,
  FolderGit2,
  Laptop,
  PencilLine,
  Server,
} from "lucide-react-native";
import React, { useEffect, useMemo, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

type PickerMode = "choices" | "browse" | "manual";

export interface ProjectPickerProps {
  value: string;
  onChange: (path: string) => void;
  error?: string;
}

export function ProjectPicker({ value, onChange, error }: ProjectPickerProps) {
  const tokens = useTokens();
  const { baseUrl } = useAuth();
  const projects = useProjects();
  const [visible, setVisible] = useState(false);
  const [mode, setMode] = useState<PickerMode>("choices");
  const [browsePath, setBrowsePath] = useState<string | undefined>();
  const [manualPath, setManualPath] = useState(value);
  const [nativeError, setNativeError] = useState<string | null>(null);
  const browser = useBrowseProjects(browsePath, visible && mode === "browse");

  const choices = useMemo(
    () => projects.data ? projectChoices(projects.data.default_cwd, projects.data.recent) : [],
    [projects.data],
  );

  useEffect(() => {
    if (visible) setManualPath(value);
  }, [visible, value]);

  const select = (path: string) => {
    onChange(path);
    setVisible(false);
    setMode("choices");
  };

  const openBrowser = (path?: string) => {
    setBrowsePath(path);
    setMode("browse");
  };

  const chooseOnDesktop = async () => {
    setNativeError(null);
    try {
      const path = await chooseNativeFolder();
      if (path) select(path);
    } catch (cause) {
      setNativeError(cause instanceof Error ? cause.message : "Folder picker failed");
    }
  };

  const selectedName = value ? projectName(value) : "Current project";

  return (
    <View style={styles.wrap}>
      <Text style={[typeScale.meta, { color: tokens.ink3 }]}>Project</Text>
      {projects.isLoading && !value ? (
        <Skeleton height={58} width="100%" />
      ) : (
        <Card padded={false}>
          <ListRow
            title={selectedName}
            subtitle={value || "Uses the server's current project"}
            leading={<FolderGit2 size={20} color={tokens.accent} />}
            trailing={<ChevronRight size={18} color={tokens.ink3} />}
            onPress={() => {
              setMode("choices");
              setVisible(true);
            }}
            accessibilityLabel={`Project: ${selectedName}. Change project`}
            showSeparator={false}
          />
        </Card>
      )}
      {error ? <Text accessibilityRole="alert" style={[typeScale.sub, { color: tokens.danger }]}>{error}</Text> : null}
      {projects.isError ? <Text style={[typeScale.sub, { color: tokens.danger }]}>Could not load recent projects. Manual entry is still available.</Text> : null}

      <Sheet visible={visible} onClose={() => setVisible(false)} accessibilityLabel="Choose project" snapPoints={[0.85]}>
        {mode === "choices" ? (
          <View style={styles.sheetContent}>
            <Text style={[typeScale.heading, { color: tokens.ink }]}>Choose project</Text>
            <Text style={[typeScale.sub, { color: tokens.ink3 }]}>Pick a recent project or browse folders on the Forge server.</Text>
            <SectionHeader>Find another project</SectionHeader>
            <Card padded={false}>
              <ListRow
                title="Browse this Forge server"
                subtitle="Only configured project roots are visible"
                leading={<Server size={20} color={tokens.ink2} />}
                trailing={<ChevronRight size={18} color={tokens.ink3} />}
                onPress={() => openBrowser(projects.data?.roots[0]?.path)}
              />
              {canChooseNativeFolder(baseUrl) ? (
                <ListRow
                  title="Choose a folder on this computer"
                  subtitle="Opens the native desktop folder picker"
                  leading={<Laptop size={20} color={tokens.ink2} />}
                  trailing={<ChevronRight size={18} color={tokens.ink3} />}
                  onPress={() => void chooseOnDesktop()}
                />
              ) : null}
              <ListRow
                title="Enter a path manually"
                subtitle="Advanced fallback for a known server path"
                leading={<PencilLine size={20} color={tokens.ink2} />}
                trailing={<ChevronRight size={18} color={tokens.ink3} />}
                onPress={() => setMode("manual")}
                showSeparator={false}
              />
            </Card>
            {nativeError ? <Text accessibilityRole="alert" style={[typeScale.sub, { color: tokens.danger }]}>{nativeError}</Text> : null}
            {choices.length > 0 ? (
              <>
                <SectionHeader>Recent projects</SectionHeader>
                <Card padded={false}>
                  {choices.map((project, index) => (
                    <ListRow
                      key={project.path}
                      title={project.path === projects.data?.default_cwd ? `${project.name} (server default)` : project.name}
                      subtitle={project.path}
                      leading={<FolderGit2 size={20} color={project.is_git_repo ? tokens.accent : tokens.ink3} />}
                      trailing={project.path === value ? <Check size={18} color={tokens.success} /> : undefined}
                      onPress={() => select(project.path)}
                      showSeparator={index < choices.length - 1}
                    />
                  ))}
                </Card>
              </>
            ) : null}
          </View>
        ) : mode === "manual" ? (
          <View style={styles.sheetContent}>
            <ListRow title="Back to projects" leading={<ArrowLeft size={18} color={tokens.ink2} />} onPress={() => setMode("choices")} showSeparator={false} />
            <Text style={[typeScale.heading, { color: tokens.ink }]}>Enter server path</Text>
            <Input
              label="Absolute working directory"
              value={manualPath}
              onChangeText={setManualPath}
              placeholder="/path/to/project"
              mono
              autoCapitalize="none"
              autoCorrect={false}
            />
            <Button label="Use this path" onPress={() => select(manualPath.trim())} disabled={!manualPath.trim()} fullWidth />
          </View>
        ) : (
          <View style={styles.sheetContent}>
            <ListRow title="Back to projects" leading={<ArrowLeft size={18} color={tokens.ink2} />} onPress={() => setMode("choices")} showSeparator={false} />
            <Text style={[typeScale.heading, { color: tokens.ink }]}>Browse server folders</Text>
            {browser.data ? <Text style={[typeScale.codeSmall, { color: tokens.ink2 }]}>{browser.data.path}</Text> : null}
            {browser.isLoading ? [0, 1, 2].map((index) => <Skeleton key={index} height={56} width="100%" />) : null}
            {browser.isError ? (
              <View style={styles.errorBlock}>
                <Text accessibilityRole="alert" style={[typeScale.sub, { color: tokens.danger }]}>
                  {browser.error instanceof ApiError ? browser.error.message : "Could not browse this folder"}
                </Text>
                <Button label="Retry" variant="secondary" onPress={() => void browser.refetch()} />
              </View>
            ) : null}
            {browser.data ? (
              <>
                <Button label={`Use ${projectName(browser.data.path)}`} onPress={() => select(browser.data.path)} fullWidth />
                <Card padded={false}>
                  {browser.data.parent ? (
                    <ListRow title="Parent folder" subtitle={browser.data.parent} leading={<ArrowUp size={18} color={tokens.ink2} />} onPress={() => setBrowsePath(browser.data?.parent ?? undefined)} />
                  ) : null}
                  {browser.data.entries.map((entry, index) => (
                    <ListRow
                      key={entry.path}
                      title={entry.name}
                      subtitle={entry.is_git_repo ? "Git project" : undefined}
                      leading={entry.is_git_repo ? <FolderGit2 size={20} color={tokens.accent} /> : <Folder size={20} color={tokens.ink3} />}
                      trailing={<ChevronRight size={18} color={tokens.ink3} />}
                      onPress={() => setBrowsePath(entry.path)}
                      showSeparator={index < browser.data.entries.length - 1}
                    />
                  ))}
                </Card>
                {browser.data.truncated ? <Text style={[typeScale.sub, { color: tokens.ink3 }]}>Showing the first 200 folders.</Text> : null}
              </>
            ) : null}
          </View>
        )}
      </Sheet>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { gap: space.space4 },
  sheetContent: { paddingHorizontal: space.space16, paddingBottom: space.space32, gap: space.space12 },
  errorBlock: { gap: space.space8 },
});
