// Machined git review dock — the commit box at the foot of the file column (docs/design/
// machined "Forge Machined - Desktop.dc.html" L265-308): message well, "Commit N files", and
// the `⑂ branch → base` target in mono.
//
// N is the staged-row count from `GET /api/git/status`; the daemon commits ONLY what is
// staged (no `-a`), so that number is the literal contract. Both disabled reasons are worded
// exactly as the daemon's own 400s ("nothing staged to commit", "a commit message is
// required") so the button's caption and a server rejection never disagree.
import React from "react";
import { Pressable, StyleSheet, Text, TextInput, View } from "react-native";

import { type GitCommitResponse } from "../../lib/api";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale, webInputTextStyle } from "../../theme/typography";

export interface GitCommitBoxProps {
  branch: string;
  baseBranch: string | null;
  stagedCount: number;
  message: string;
  onChangeMessage: (message: string) => void;
  onCommit: () => void;
  committing: boolean;
  error: Error | null;
  /** The last successful commit, echoed back as its short sha + subject. */
  result: GitCommitResponse | null;
}

const SHORT_SHA = 7;

export function GitCommitBox({
  branch,
  baseBranch,
  stagedCount,
  message,
  onChangeMessage,
  onCommit,
  committing,
  error,
  result,
}: GitCommitBoxProps) {
  const tokens = useTokens();
  const trimmed = message.trim();
  const blockedReason =
    stagedCount === 0
      ? "nothing staged to commit"
      : trimmed.length === 0
        ? "a commit message is required"
        : null;
  const disabled = blockedReason != null || committing;

  return (
    <View style={[styles.box, { borderTopColor: tokens.border }]}>
      <TextInput
        value={message}
        onChangeText={onChangeMessage}
        multiline
        numberOfLines={3}
        editable={!committing}
        placeholder="commit message"
        placeholderTextColor={tokens.ink3}
        accessibilityLabel="Commit message"
        style={[
          styles.input,
          typeScale.body,
          webInputTextStyle,
          {
            backgroundColor: tokens.bg0,
            borderColor: tokens.border,
            borderRadius: radii.radius4,
            color: tokens.ink,
          },
        ]}
      />

      <Pressable
        onPress={disabled ? undefined : onCommit}
        disabled={disabled}
        accessibilityRole="button"
        accessibilityState={{ disabled, busy: committing }}
        accessibilityLabel={
          blockedReason
            ? `Commit unavailable — ${blockedReason}`
            : `Commit ${stagedCount} file${stagedCount === 1 ? "" : "s"}`
        }
        style={[
          styles.button,
          {
            // ds/Button's disabled convention (flat bg3 fill + ink4 label) at dock density —
            // its own 48px primary height is twice this row.
            backgroundColor: disabled ? tokens.bg3 : tokens.accent,
            borderRadius: radii.radius4,
          },
        ]}
      >
        <Text style={[typeScale.meta, { color: disabled ? tokens.ink4 : tokens.onAccent }]}>
          {committing ? "Committing…" : `Commit ${stagedCount} file${stagedCount === 1 ? "" : "s"}`}
        </Text>
      </Pressable>

      {blockedReason && !committing ? (
        <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>{blockedReason}</Text>
      ) : null}
      {error ? <Text style={[typeScale.monoMeta, { color: tokens.danger }]}>{error.message}</Text> : null}
      {result && !error ? (
        <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.success }]} numberOfLines={2}>
          ✓ {result.sha.slice(0, SHORT_SHA)} {result.summary}
        </Text>
      ) : null}

      <Text style={[typeScale.monoMeta, styles.branch, { color: tokens.ink3 }]} numberOfLines={1}>
        ⑂ {branch || "detached HEAD"}
        {baseBranch ? ` → ${baseBranch}` : ""}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  box: {
    borderTopWidth: StyleSheet.hairlineWidth,
    padding: 10,
    gap: 7,
  },
  input: {
    borderWidth: 1,
    minHeight: 54,
    paddingHorizontal: space.space8,
    paddingVertical: 6,
    textAlignVertical: "top",
  },
  button: { height: 28, alignItems: "center", justifyContent: "center" },
  branch: { fontFamily: monoFamily.regular },
});
