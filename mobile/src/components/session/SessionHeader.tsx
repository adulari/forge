// Session shell header: focused controls plus an accessible overflow for less-frequent actions.
import {
  ArrowLeft,
  BookOpen,
  Bookmark,
  Bot,
  Brain,
  GitFork,
  GitPullRequest,
  History,
  Map,
  Microscope,
  MoreHorizontal,
  Network,
  Plus,
  Search,
  Swords,
  Workflow,
} from "lucide-react-native";
import React, { useCallback, useState } from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space, type StatusDotState } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { IconButton } from "../ds/IconButton";
import { ListRow } from "../ds/ListRow";
import { Sheet } from "../ds/Sheet";
import { StatusDot } from "../ds/StatusDot";

export interface SessionHeaderProps {
  title: string;
  /** Hearth core rule 3: a waiting/busy session carries its Emberdot beside the title —
   * the header is the "is this alive / does it need me" glance, not a separate row. */
  state: StatusDotState;
  /** Only read for the public-exposure safety tag — repo/branch/model now live in the
   * mono meta line StatusStrip renders directly under this header (Hearth core rule 8:
   * "session titles are task titles ... repo · branch · model live in the mono meta line"). */
  exposure: string;
  onBack: () => void;
  showBack?: boolean;
  onNewHere: () => void;
  onPalette: () => void;
  onDuel: () => void;
  onReplay: () => void;
  onPlan: () => void;
  onWorkflows: () => void;
  onFork: () => void;
  onInit: () => void;
  onAssay: () => void;
  onSelfMcp: () => void;
  onCheckpoint: () => void;
  onPullRequest: () => void;
  onMemory: () => void;
  onLattice: () => void;
}

export function SessionHeader(props: SessionHeaderProps) {
  const tokens = useTokens();
  const [actionsVisible, setActionsVisible] = useState(false);
  const isPublic = props.exposure.startsWith("public");
  const closeActions = useCallback(() => setActionsVisible(false), []);
  const run = useCallback((action: () => void) => {
    closeActions();
    action();
  }, [closeActions]);

  return (
    <View style={styles.wrap}>
      <View style={styles.row}>
        {props.showBack !== false ? <IconButton icon={<ArrowLeft size={20} strokeWidth={1.75} color={tokens.ink} />} onPress={props.onBack} accessibilityLabel="Back" /> : null}
        <StatusDot state={props.state} />
        <Text style={[typeScale.headingBold, styles.title, { color: tokens.ink }]} numberOfLines={1}>{props.title}</Text>
        {isPublic ? <Text style={[typeScale.meta, { color: tokens.danger }]}>public</Text> : null}
        <IconButton icon={<Search size={20} strokeWidth={1.75} color={tokens.ink} />} onPress={props.onPalette} accessibilityLabel="Open command palette" />
        <IconButton icon={<MoreHorizontal size={20} strokeWidth={1.75} color={tokens.ink} />} onPress={() => setActionsVisible(true)} accessibilityLabel="Session actions" />
      </View>
      <Sheet visible={actionsVisible} onClose={closeActions} accessibilityLabel="Session actions" snapPoints={[0.8]}>
        <ScrollView contentContainerStyle={styles.actions} keyboardShouldPersistTaps="handled">
          <Text style={[typeScale.heading, { color: tokens.ink }]}>Session actions</Text>
          <ListRow title="Start another session here" leading={<Plus size={20} color={tokens.ink2} />} onPress={() => run(props.onNewHere)} />
          <ListRow title="Start model duel" leading={<Swords size={20} color={tokens.ink2} />} onPress={() => run(props.onDuel)} />
          <ListRow title="Open session replay" leading={<History size={20} color={tokens.ink2} />} onPress={() => run(props.onReplay)} />
          <ListRow title="Open workflows" leading={<Workflow size={20} color={tokens.ink2} />} onPress={() => run(props.onWorkflows)} />
          <ListRow title="Create implementation plan" leading={<Map size={20} color={tokens.ink2} />} onPress={() => run(props.onPlan)} />
          <ListRow title="Fork session" leading={<GitFork size={20} color={tokens.ink2} />} onPress={() => run(props.onFork)} />
          <ListRow title="Initialize project guidance" leading={<BookOpen size={20} color={tokens.ink2} />} onPress={() => run(props.onInit)} />
          <ListRow title="Run quality assay" leading={<Microscope size={20} color={tokens.ink2} />} onPress={() => run(props.onAssay)} />
          <ListRow title="Manage self MCP agent" leading={<Bot size={20} color={tokens.ink2} />} onPress={() => run(props.onSelfMcp)} />
          <ListRow title="Manage session checkpoints" leading={<Bookmark size={20} color={tokens.ink2} />} onPress={() => run(props.onCheckpoint)} />
          <ListRow title="Create pull request" leading={<GitPullRequest size={20} color={tokens.ink2} />} onPress={() => run(props.onPullRequest)} />
          <ListRow title="Manage project memory" leading={<Brain size={20} color={tokens.ink2} />} onPress={() => run(props.onMemory)} />
          <ListRow title="Inspect code symbol" leading={<Network size={20} color={tokens.ink2} />} onPress={() => run(props.onLattice)} showSeparator={false} />
        </ScrollView>
      </Sheet>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { gap: space.space4 },
  row: { flexDirection: "row", alignItems: "center", gap: space.space8, minHeight: 44 },
  title: { flex: 1 },
  actions: { paddingHorizontal: space.space16, paddingBottom: space.space32, gap: space.space4 },
});
