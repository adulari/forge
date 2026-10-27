// Header of a subagent child session: the breadcrumb back to the parent ("parent › explore"), the
// child's own state dot, and a mono meta line. It replaces the interactive SessionHeader and
// StatusStrip, whose controls (model, temper, actions menu) address a live driver the child lacks.
import { ArrowLeft, ChevronRight } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { space, type StatusDotState } from "../../theme/tokens";
import { formatCost, tabularNums, type as typeScale } from "../../theme/typography";
import { IconButton } from "../ds/IconButton";
import { StatusDot } from "../ds/StatusDot";

export interface ChildSessionHeaderProps {
  title: string;
  parentTitle: string;
  state: StatusDotState;
  model: string | null;
  costUsd: number;
  showBack: boolean;
  onBack: () => void;
  onOpenParent: () => void;
}

function ChildSessionHeaderBase(props: ChildSessionHeaderProps) {
  const tokens = useTokens();
  const parent = props.parentTitle || "parent session";
  const model = props.model && props.model !== "—" ? props.model : null;
  const meta = ["subagent", model, formatCost(props.costUsd)].filter(Boolean).join(" · ");
  return (
    <View style={styles.wrap} testID="child-session-header">
      <View style={styles.row}>
        {props.showBack ? (
          <IconButton
            icon={<ArrowLeft size={20} strokeWidth={1.75} color={tokens.ink} />}
            onPress={props.onBack}
            accessibilityLabel="Back to parent session"
            style={styles.leadingBleed}
          />
        ) : null}
        <Pressable
          onPress={props.onOpenParent}
          accessibilityRole="link"
          accessibilityLabel={`Open parent session ${parent}`}
          style={styles.crumb}
        >
          <Text style={[typeScale.body, { color: tokens.ink3 }]} numberOfLines={1}>
            {parent}
          </Text>
        </Pressable>
        <ChevronRight size={15} strokeWidth={1.75} color={tokens.ink4} />
        <StatusDot state={props.state} />
        <Text style={[typeScale.headingBold, styles.title, { color: tokens.ink }]} numberOfLines={1}>
          {props.title}
        </Text>
      </View>
      <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3 }]} numberOfLines={1}>
        {meta}
      </Text>
    </View>
  );
}

export const ChildSessionHeader = React.memo(ChildSessionHeaderBase);

const styles = StyleSheet.create({
  wrap: { gap: space.space2, paddingBottom: space.space8 },
  row: { flexDirection: "row", alignItems: "center", gap: space.space8, minHeight: 44 },
  crumb: { flexShrink: 1, maxWidth: "40%" },
  title: { flexShrink: 1 },
  leadingBleed: { marginLeft: -space.space12 },
});
