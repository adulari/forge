// Session shell header (T3.1, DESIGN_SYSTEM.md §6): back control, title, cwd (mono,
// head-ellipsized), worktree Badge, exposure Badge. The danger Banner for public exposure
// is rendered by the shell itself (_layout.tsx) — this component owns the header row only.
import {
  ArrowLeft,
  Search,
  Swords,
  History,
  Map,
  GitFork,
  BookOpen,
  Microscope,
  Bot,
  Bookmark,
  GitPullRequest,
  Brain,
  Network,
} from "lucide-react-native";
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { IconButton } from "../ds/IconButton";

export interface SessionHeaderProps {
  title: string;
  cwd: string;
  worktree: string | null;
  exposure: string;
  onBack: () => void;
  onPalette: () => void;
  onDuel: () => void;
  onReplay: () => void;
  onPlan: () => void;
  onFork: () => void;
  onInit: () => void;
  onAssay: () => void;
  onSelfMcp: () => void;
  onCheckpoint: () => void;
  onPullRequest: () => void;
  onMemory: () => void;
  onLattice: () => void;
}

export function SessionHeader({
  title,
  cwd,
  worktree,
  exposure,
  onBack,
  onPalette,
  onDuel,
  onReplay,
  onPlan,
  onFork,
  onInit,
  onAssay,
  onSelfMcp,
  onCheckpoint,
  onPullRequest,
  onMemory,
  onLattice,
}: SessionHeaderProps) {
  const tokens = useTokens();
  const isPublic = exposure.startsWith("public");

  return (
    <View style={styles.wrap}>
      <View style={styles.row}>
        <IconButton icon={<ArrowLeft size={20} strokeWidth={1.75} color={tokens.ink} />} onPress={onBack} accessibilityLabel="Back" />
        <IconButton icon={<Swords size={20} strokeWidth={1.75} color={tokens.ink} />} onPress={onDuel} accessibilityLabel="Start model duel" />
        <IconButton icon={<History size={20} strokeWidth={1.75} color={tokens.ink} />} onPress={onReplay} accessibilityLabel="Open session replay" />
        <IconButton icon={<Map size={20} strokeWidth={1.75} color={tokens.ink} />} onPress={onPlan} accessibilityLabel="Create implementation plan" />
        <IconButton icon={<GitFork size={20} strokeWidth={1.75} color={tokens.ink} />} onPress={onFork} accessibilityLabel="Fork session" />
        <IconButton icon={<BookOpen size={20} strokeWidth={1.75} color={tokens.ink} />} onPress={onInit} accessibilityLabel="Initialize project guidance" />
        <IconButton icon={<Microscope size={20} strokeWidth={1.75} color={tokens.ink} />} onPress={onAssay} accessibilityLabel="Run quality assay" />
        <IconButton icon={<Bot size={20} strokeWidth={1.75} color={tokens.ink} />} onPress={onSelfMcp} accessibilityLabel="Manage self MCP agent" />
        <IconButton icon={<Bookmark size={20} strokeWidth={1.75} color={tokens.ink} />} onPress={onCheckpoint} accessibilityLabel="Manage session checkpoints" />
        <IconButton icon={<GitPullRequest size={20} strokeWidth={1.75} color={tokens.ink} />} onPress={onPullRequest} accessibilityLabel="Create pull request" />
        <IconButton icon={<Brain size={20} strokeWidth={1.75} color={tokens.ink} />} onPress={onMemory} accessibilityLabel="Manage project memory" />
        <IconButton icon={<Network size={20} strokeWidth={1.75} color={tokens.ink} />} onPress={onLattice} accessibilityLabel="Inspect code symbol" />
        <IconButton icon={<Search size={20} strokeWidth={1.75} color={tokens.ink} />} onPress={onPalette} accessibilityLabel="Open command palette" />
        <Text style={[typeScale.heading, styles.title, { color: tokens.ink }]} numberOfLines={1}>
          {title}
        </Text>
        {worktree ? <Badge label="worktree" tone="outline" /> : null}
        <Badge label={exposure} tone={isPublic ? "danger" : "neutral"} />
      </View>
      <Text
        style={[typeScale.codeSmall, styles.cwd, { color: tokens.ink2 }]}
        numberOfLines={1}
        ellipsizeMode="head"
      >
        {cwd}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { gap: space.space4 },
  row: { flexDirection: "row", alignItems: "center", gap: space.space8, minHeight: 44 },
  title: { flex: 1 },
  // Aligns under the title text, past the 44pt back-button hit area.
  cwd: { paddingLeft: 44 + space.space8 },
});
