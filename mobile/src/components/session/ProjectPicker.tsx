// Hearth "Choose project" sheet (mobile.dc.html:435) — de-boxed lists (core rule 1):
// the trigger row and every list inside the sheet are plain hairline-separated
// ListRows, never wrapped in a Card. "Recent" comes before "Find another" per spec.
import { ApiError, type ProjectRow } from "../../lib/api";
import { offersGitInit, type ProjectManageDraft } from "../../lib/projectOnboarding";
import { useHomeProject } from "../../lib/queriesProjects";
import { canChooseNativeFolder, chooseNativeFolder } from "../../lib/folderPicker";
import { projectChoices, projectName } from "../../lib/projectSelection";
import { useBrowseProjects, useProjects } from "../../lib/queries";
import { useAuth } from "../../lib/auth";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { IconButton } from "../ds/IconButton";
import { Input } from "../ds/Input";
import { ListRow } from "../ds/ListRow";
import { SectionHeader } from "../ds/SectionHeader";
import { Sheet } from "../ds/Sheet";
import { Skeleton } from "../ds/Skeleton";
import { CloneRepoPanel } from "./CloneRepoPanel";
import { GitInitOffer } from "./GitInitOffer";
import { ProjectAdminSection } from "./ProjectAdminSection";
import { ProjectManagePanel } from "./ProjectManagePanel";
import {
  ArrowLeft,
  ArrowUp,
  Check,
  ChevronRight,
  Folder,
  FolderGit2,
  GitFork,
  Home,
  Laptop,
  MoreHorizontal,
  PencilLine,
  Server,
} from "lucide-react-native";
import React, { useEffect, useMemo, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

type PickerMode = "choices" | "browse" | "manual" | "clone" | "manage";

export interface ProjectPickerProps {
  value: string;
  onChange: (path: string) => void;
  error?: string;
  /** Opens the sheet straight into a flow (the command palette's "Clone repository"). */
  initialAction?: "clone" | "manage";
}

interface ManageState {
  project: ProjectRow;
  hidden: boolean;
  draft: ProjectManageDraft;
}

export function ProjectPicker({ value, onChange, error, initialAction }: ProjectPickerProps) {
  const tokens = useTokens();
  const { baseUrl } = useAuth();
  const projects = useProjects();
  const home = useHomeProject();
  const [visible, setVisible] = useState(initialAction != null);
  const [mode, setMode] = useState<PickerMode>(initialAction === "clone" ? "clone" : "choices");
  const [manage, setManage] = useState<ManageState | null>(null);
  const [relocating, setRelocating] = useState(false);
  const [seenRepoState, setSeenRepoState] = useState<Record<string, boolean>>({});
  const [browsePath, setBrowsePath] = useState<string | undefined>();
  const [manualPath, setManualPath] = useState(value);
  const [nativeError, setNativeError] = useState<string | null>(null);
  const browser = useBrowseProjects(browsePath, visible && mode === "browse");

  const choices = useMemo(
    () => (projects.data ? projectChoices(projects.data.default_cwd, projects.data.recent, projects.data.roots) : []),
    [projects.data],
  );

  useEffect(() => {
    if (visible) setManualPath(value);
  }, [visible, value]);

  useEffect(() => {
    if (!browser.data) return;
    setSeenRepoState((seen) => {
      const next = { ...seen };
      for (const entry of browser.data.entries) next[entry.path] = entry.is_git_repo;
      return next;
    });
  }, [browser.data]);

  const openManage = (project: ProjectRow, hidden: boolean) => {
    setManage({ project, hidden, draft: { name: project.name, relocate: "" } });
    setMode("manage");
  };

  const pickFolder = (path: string) => {
    if (relocating && manage) {
      setManage({ ...manage, draft: { ...manage.draft, relocate: path } });
      setRelocating(false);
      setMode("manage");
      return;
    }
    select(path);
  };

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

  const isHome = !!value && value === home.data?.path;
  const selectedName = isHome ? "No project (Home)" : value ? projectName(value) : "Current project";

  return (
    <View style={styles.wrap}>
      {projects.isLoading && !value ? (
        <Skeleton height={56} width="100%" />
      ) : (
        <ListRow
          title={selectedName}
          subtitle={isHome ? "A scratch folder, no repository" : value || "Uses the server's current project"}
          leading={isHome ? <Home size={18} strokeWidth={1.75} color={tokens.accent} /> : <FolderGit2 size={18} strokeWidth={1.75} color={tokens.accent} />}
          trailing={<ChevronRight size={16} strokeWidth={1.75} color={tokens.ink3} />}
          onPress={() => {
            setMode("choices");
            setVisible(true);
          }}
          accessibilityLabel={`Project: ${selectedName}. Change project`}
        />
      )}
      {error ? (
        <Text accessibilityRole="alert" style={[typeScale.sub, styles.errorText, { color: tokens.danger }]}>
          {error}
        </Text>
      ) : null}
      {projects.isError ? (
        <Text style={[typeScale.sub, styles.errorText, { color: tokens.danger }]}>
          Could not load recent projects. Manual entry is still available.
        </Text>
      ) : null}

      <Sheet visible={visible} onClose={() => {
          setVisible(false);
          setRelocating(false);
        }} accessibilityLabel="Choose project" snapPoints={[0.85]}>
        {mode === "choices" ? (
          <View style={styles.sheetContent}>
            <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Choose project</Text>
            <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
              Pick a recent project or browse the Forge server.
            </Text>

            {home.data ? (
              <ListRow
                title="No project (Home)"
                subtitle="Chat and scratch work without a repository"
                leading={<Home size={18} strokeWidth={1.75} color={tokens.accent} />}
                trailing={home.data.path === value ? <Check size={16} strokeWidth={2} color={tokens.success} /> : undefined}
                onPress={() => select(home.data.path)}
                showSeparator={false}
              />
            ) : null}

            {choices.length > 0 ? (
              <View>
                <SectionHeader>Recent</SectionHeader>
                {choices.map((project, index) => (
                  <ListRow
                    key={project.path}
                    title={
                      project.path === projects.data?.default_cwd
                        ? `${project.name} · server default`
                        : project.name
                    }
                    subtitle={project.path}
                    leading={
                      <FolderGit2
                        size={18}
                        strokeWidth={1.75}
                        color={project.is_git_repo ? tokens.accent : tokens.ink3}
                      />
                    }
                    trailing={
                      <View style={styles.trailingRow}>
                        {project.path === value ? <Check size={16} strokeWidth={2} color={tokens.success} /> : null}
                        <IconButton
                          icon={<MoreHorizontal size={18} strokeWidth={1.75} color={tokens.ink3} />}
                          onPress={() => openManage(project, false)}
                          accessibilityLabel={`Manage ${project.name}`}
                        />
                      </View>
                    }
                    hasInteractiveTrailing
                    onPress={() => select(project.path)}
                    showSeparator={index < choices.length - 1}
                  />
                ))}
              </View>
            ) : null}

            <View>
              <SectionHeader>Find another</SectionHeader>
              <ListRow
                title="Browse this Forge server"
                subtitle="Only configured project roots are visible"
                leading={<Server size={18} strokeWidth={1.75} color={tokens.ink2} />}
                trailing={<ChevronRight size={16} strokeWidth={1.75} color={tokens.ink3} />}
                onPress={() => openBrowser(projects.data?.roots[0]?.path)}
              />
              {canChooseNativeFolder(baseUrl) ? (
                <ListRow
                  title="Choose a folder on this computer"
                  subtitle="Opens the native desktop folder picker"
                  leading={<Laptop size={18} strokeWidth={1.75} color={tokens.ink2} />}
                  trailing={<ChevronRight size={16} strokeWidth={1.75} color={tokens.ink3} />}
                  onPress={() => void chooseOnDesktop()}
                />
              ) : null}
              <ListRow
                title="Clone a repository"
                subtitle="From a URL or your GitHub repositories"
                leading={<GitFork size={18} strokeWidth={1.75} color={tokens.ink2} />}
                trailing={<ChevronRight size={16} strokeWidth={1.75} color={tokens.ink3} />}
                onPress={() => setMode("clone")}
              />
              <ListRow
                title="Enter a path manually"
                subtitle="Advanced fallback for a known server path"
                leading={<PencilLine size={18} strokeWidth={1.75} color={tokens.ink2} />}
                trailing={<ChevronRight size={16} strokeWidth={1.75} color={tokens.ink3} />}
                onPress={() => setMode("manual")}
                showSeparator={false}
              />
            </View>
            <ProjectAdminSection
              hidden={projects.data?.hidden ?? []}
              missing={projects.data?.missing ?? []}
              onManageHidden={(project) => openManage(project, true)}
              onManageMissing={(project) => openManage(project, false)}
            />
            {nativeError ? (
              <Text accessibilityRole="alert" style={[typeScale.sub, { color: tokens.danger }]}>
                {nativeError}
              </Text>
            ) : null}
          </View>
        ) : mode === "clone" ? (
          <CloneRepoPanel onBack={() => setMode("choices")} onCloned={select} />
        ) : mode === "manage" && manage ? (
          <ProjectManagePanel
            project={manage.project}
            hidden={manage.hidden}
            draft={manage.draft}
            onDraftChange={(draft) => setManage({ ...manage, draft })}
            onBack={() => setMode("choices")}
            onBrowseRelocate={() => {
              setRelocating(true);
              openBrowser(projects.data?.roots[0]?.path);
            }}
          />
        ) : mode === "manual" ? (
          <View style={styles.sheetContent}>
            <ListRow title="Back to projects" leading={<ArrowLeft size={16} strokeWidth={1.75} color={tokens.ink2} />} onPress={() => setMode("choices")} showSeparator={false} />
            <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Enter server path</Text>
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
            <ListRow
              title={relocating ? "Back to project" : "Back to projects"}
              leading={<ArrowLeft size={16} strokeWidth={1.75} color={tokens.ink2} />}
              onPress={() => {
                setMode(relocating ? "manage" : "choices");
                setRelocating(false);
              }}
              showSeparator={false}
            />
            <Text style={[typeScale.headingBold, { color: tokens.ink }]}>
              {relocating ? "Choose the new location" : "Browse server folders"}
            </Text>
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
                <Button
                  label={relocating ? `Move to ${projectName(browser.data.path)}` : `Use ${projectName(browser.data.path)}`}
                  onPress={() => pickFolder(browser.data.path)}
                  fullWidth
                />
                {!relocating && offersGitInit(seenRepoState[browser.data.path], browser.data.path) ? (
                  <GitInitOffer
                    path={browser.data.path}
                    onInitialized={() => setSeenRepoState((seen) => ({ ...seen, [browser.data.path]: true }))}
                  />
                ) : null}
                <View>
                  {browser.data.parent ? (
                    <ListRow title="Parent folder" subtitle={browser.data.parent} leading={<ArrowUp size={18} strokeWidth={1.75} color={tokens.ink2} />} onPress={() => setBrowsePath(browser.data?.parent ?? undefined)} />
                  ) : null}
                  {browser.data.entries.map((entry, index) => (
                    <ListRow
                      key={entry.path}
                      title={entry.name}
                      subtitle={entry.is_git_repo ? "Git project" : undefined}
                      leading={entry.is_git_repo ? <FolderGit2 size={18} strokeWidth={1.75} color={tokens.accent} /> : <Folder size={18} strokeWidth={1.75} color={tokens.ink3} />}
                      trailing={<ChevronRight size={16} strokeWidth={1.75} color={tokens.ink3} />}
                      onPress={() => setBrowsePath(entry.path)}
                      showSeparator={index < browser.data.entries.length - 1}
                    />
                  ))}
                </View>
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
  trailingRow: { flexDirection: "row", alignItems: "center", gap: space.space4 },
  errorText: { paddingHorizontal: space.space16 },
});
