// A foldable notice that sits on the composer like the plan and spend docks: a headline that is
// always visible and details behind a chevron, so a long provider error does not take the screen.
import { ChevronDown, ChevronRight, X } from "lucide-react-native";
import React, { useState } from "react";
import { StyleSheet, Text, View } from "react-native";
import type { LucideIcon } from "lucide-react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";
import { MiniPressable } from "../chat/MiniPressable";

export interface NoticeDockProps {
  tone: "danger" | "warn";
  icon: LucideIcon;
  title: string;
  /** One line shown beside the title. */
  summary: string;
  /** Shown when unfolded; omit when the summary already says everything. */
  details?: string;
  onDismiss: () => void;
  dismissLabel: string;
  testID: string;
}

export function NoticeDock({ tone, icon: Icon, title, summary, details, onDismiss, dismissLabel, testID }: NoticeDockProps) {
  const tokens = useTokens();
  const [open, setOpen] = useState(false);
  const accent = tone === "danger" ? tokens.danger : tokens.warn;
  const foldable = details != null && details !== "";
  const Chevron = open ? ChevronDown : ChevronRight;
  return (
    <View
      style={[styles.frame, { backgroundColor: tokens.bg1, borderColor: tokens.border, borderLeftColor: accent }]}
      testID={testID}
    >
      <View style={styles.header}>
        <MiniPressable
          onPress={foldable ? () => setOpen((value) => !value) : undefined}
          disabled={!foldable}
          accessibilityRole={foldable ? "button" : "text"}
          accessibilityState={foldable ? { expanded: open } : undefined}
          accessibilityLabel={`${title}. ${summary}${foldable ? (open ? ". Hide details" : ". Show details") : ""}`}
          wash={false}
          style={styles.toggle}
          testID={`${testID}-toggle`}
        >
          {foldable ? <Chevron size={13} strokeWidth={1.75} color={tokens.ink3} /> : null}
          <Icon size={13} strokeWidth={1.75} color={accent} />
          <Text style={[typeScale.meta, { color: tokens.ink }]}>{title}</Text>
          <Text style={[typeScale.sub, styles.summary, { color: tokens.ink3 }]} numberOfLines={open ? undefined : 1}>
            {summary}
          </Text>
        </MiniPressable>
        <MiniPressable
          onPress={onDismiss}
          accessibilityRole="button"
          accessibilityLabel={dismissLabel}
          hitSlop={{ top: 6, bottom: 6, left: 4, right: 4 }}
          style={styles.dismiss}
          testID={`${testID}-dismiss`}
        >
          <X size={14} strokeWidth={1.75} color={tokens.ink3} />
        </MiniPressable>
      </View>
      {open && foldable ? (
        <Text selectable style={[styles.details, { color: tokens.ink2, backgroundColor: tokens.bg0 }]}>
          {details}
        </Text>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  frame: {
    marginHorizontal: space.space12,
    marginBottom: space.space4,
    borderWidth: StyleSheet.hairlineWidth * 2,
    borderLeftWidth: 3,
    borderRadius: radii.radius8,
    overflow: "hidden",
  },
  header: { flexDirection: "row", alignItems: "center" },
  toggle: {
    flex: 1,
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingLeft: space.space8,
    paddingVertical: space.space4,
    minHeight: 30,
  },
  summary: { flex: 1, minWidth: 0 },
  dismiss: { paddingHorizontal: space.space8, paddingVertical: space.space4, minHeight: 30, justifyContent: "center" },
  details: {
    fontFamily: monoFamily.regular,
    fontSize: 11.5,
    lineHeight: 16,
    paddingHorizontal: space.space12,
    paddingVertical: space.space8,
    maxHeight: 180,
  },
});
