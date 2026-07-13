// Session shell header (T3.1, DESIGN_SYSTEM.md §6): back control, title, cwd (mono,
// head-ellipsized), worktree Badge, exposure Badge. The danger Banner for public exposure
// is rendered by the shell itself (_layout.tsx) — this component owns the header row only.
import { ArrowLeft, Search, Swords, History, Map } from "lucide-react-native";
||||||| parent of d65aed8 (feat(mobile): fork sessions from history)
import { ArrowLeft, Search } from "lucide-react-native";

import { ArrowLeft, Search, GitFork } from "lucide-react-native";
||||||| parent of a8da0b2 (feat(mobile): initialize project guidance)
import { ArrowLeft, Search } from "lucide-react-native";

import { ArrowLeft, Search, BookOpen } from "lucide-react-native";
||||||| parent of 86f0b39 (feat(mobile): run quality assays from sessions)
import { ArrowLeft, Search } from "lucide-react-native";

import { ArrowLeft, Search, Microscope } from "lucide-react-native";
||||||| parent of 4788709 (feat(mobile): manage self MCP agents)
import { ArrowLeft, Search } from "lucide-react-native";

import { ArrowLeft, Search, Bot } from "lucide-react-native";
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
||||||| parent of d65aed8 (feat(mobile): fork sessions from history)

  onFork: () => void;
||||||| parent of a8da0b2 (feat(mobile): initialize project guidance)

  onInit: () => void;
||||||| parent of 86f0b39 (feat(mobile): run quality assays from sessions)

  onAssay: () => void;
||||||| parent of 4788709 (feat(mobile): manage self MCP agents)

  onSelfMcp: () => void;
}

export function SessionHeader({ title, cwd, worktree, exposure, onBack, onPalette, onDuel, onReplay, onPlan }: SessionHeaderProps) {
||||||| parent of d65aed8 (feat(mobile): fork sessions from history)
export function SessionHeader({ title, cwd, worktree, exposure, onBack, onPalette }: SessionHeaderProps) {

export function SessionHeader({ title, cwd, worktree, exposure, onBack, onPalette, onFork }: SessionHeaderProps) {
||||||| parent of a8da0b2 (feat(mobile): initialize project guidance)
export function SessionHeader({ title, cwd, worktree, exposure, onBack, onPalette }: SessionHeaderProps) {

export function SessionHeader({ title, cwd, worktree, exposure, onBack, onPalette, onInit }: SessionHeaderProps) {
||||||| parent of 86f0b39 (feat(mobile): run quality assays from sessions)
export function SessionHeader({ title, cwd, worktree, exposure, onBack, onPalette }: SessionHeaderProps) {

export function SessionHeader({ title, cwd, worktree, exposure, onBack, onPalette, onAssay }: SessionHeaderProps) {
||||||| parent of 4788709 (feat(mobile): manage self MCP agents)
export function SessionHeader({ title, cwd, worktree, exposure, onBack, onPalette }: SessionHeaderProps) {

export function SessionHeader({ title, cwd, worktree, exposure, onBack, onPalette, onSelfMcp }: SessionHeaderProps) {
  const tokens = useTokens();
  const isPublic = exposure.startsWith("public");

  return (
    <View style={styles.wrap}>
      <View style={styles.row}>
        <IconButton
          icon={<ArrowLeft size={20} strokeWidth={1.75} color={tokens.ink} />}
          onPress={onBack}
          accessibilityLabel="Back"
        />
        <IconButton
          icon={<Swords size={20} strokeWidth={1.75} color={tokens.ink} />} onPress={onDuel} accessibilityLabel="Start model duel" />
        <IconButton icon={<History size={20} strokeWidth={1.75} color={tokens.ink} />} onPress={onReplay} accessibilityLabel="Open session replay" />
        <IconButton icon={<Map size={20} strokeWidth={1.75} color={tokens.ink} />} onPress={onPlan} accessibilityLabel="Create implementation plan"
||||||| parent of d65aed8 (feat(mobile): fork sessions from history)

          icon={<GitFork size={20} strokeWidth={1.75} color={tokens.ink} />}
          onPress={onFork}
          accessibilityLabel="Fork session"
||||||| parent of a8da0b2 (feat(mobile): initialize project guidance)

          icon={<BookOpen size={20} strokeWidth={1.75} color={tokens.ink} />}
          onPress={onInit}
          accessibilityLabel="Initialize project guidance"
||||||| parent of 86f0b39 (feat(mobile): run quality assays from sessions)

          icon={<Microscope size={20} strokeWidth={1.75} color={tokens.ink} />}
          onPress={onAssay}
          accessibilityLabel="Run quality assay"
||||||| parent of 4788709 (feat(mobile): manage self MCP agents)

          icon={<Bot size={20} strokeWidth={1.75} color={tokens.ink} />}
          onPress={onSelfMcp}
          accessibilityLabel="Manage self MCP agent"
        />
        <IconButton
          icon={<Search size={20} strokeWidth={1.75} color={tokens.ink} />}
          onPress={onPalette}
          accessibilityLabel="Open command palette"
        />
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
