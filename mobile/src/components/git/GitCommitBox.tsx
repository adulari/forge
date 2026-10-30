// Machined git review dock - the commit box at the foot of the file column (docs/design/
// machined "Forge Machined - Desktop.dc.html" L265-308): message well, a split commit button,
// and the `⑂ branch → base` target in mono.
//
// N is the staged-row count from `GET /api/git/status`; the daemon commits ONLY what is
// staged (no `-a`), so that number is the literal contract. Both disabled reasons are worded
// exactly as the daemon's own 400s ("nothing staged to commit", "a commit message is
// required") so the button's caption and a server rejection never disagree.
//
// The sparkle in the message well asks the daemon for a message written from the staged diff; it
// only fills the well, never commits. The primary button runs whichever of "Commit" / "Commit &
// push" was used last (remembered by the caller); its caret opens the other. Ctrl/Cmd+Enter in
// the well runs the primary action.
import { ChevronDown, Sparkles } from "lucide-react-native";
import React, { useState } from "react";
import { ActivityIndicator, Pressable, StyleSheet, Text, TextInput, View } from "react-native";

import { COMMIT_ACTION_LABEL, type CommitAction, isCommitChord } from "./gitReviewSupport";
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
  onCommit: (action: CommitAction) => void;
  committing: boolean;
  pushing: boolean;
  /** Whether "Commit & push" can mean anything (the repository has a remote). */
  canPush: boolean;
  /** The remembered primary action. */
  action: CommitAction;
  onChooseAction: (action: CommitAction) => void;
  onGenerate: () => void;
  generating: boolean;
  error: Error | null;
  /** The last successful commit, echoed back as its short sha + subject. */
  result: GitCommitResponse | null;
  /** Set once the last commit's push landed ("origin"). */
  pushedTo: string | null;
  /** Opens the pull request sheet for work that is already committed. */
  onOpenPullRequest?: () => void;
}

const SHORT_SHA = 7;

function MenuItem({
  option,
  current,
  disabled,
  onPress,
}: {
  option: CommitAction;
  current: boolean;
  disabled: boolean;
  onPress: () => void;
}) {
  const tokens = useTokens();
  const [hovered, setHovered] = useState(false);
  return (
    <Pressable
      onPress={onPress}
      onHoverIn={() => setHovered(true)}
      onHoverOut={() => setHovered(false)}
      accessibilityRole="button"
      accessibilityState={{ selected: current, disabled }}
      accessibilityLabel={COMMIT_ACTION_LABEL[option]}
      style={[styles.menuItem, { backgroundColor: hovered ? tokens.bg3 : "transparent", opacity: disabled ? 0.5 : 1 }]}
    >
      <Text style={[typeScale.meta, { color: current ? tokens.ink : tokens.ink2 }]}>{COMMIT_ACTION_LABEL[option]}</Text>
      {current ? <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>last used</Text> : null}
    </Pressable>
  );
}

export function GitCommitBox({
  branch,
  baseBranch,
  stagedCount,
  message,
  onChangeMessage,
  onCommit,
  committing,
  pushing,
  canPush,
  action,
  onChooseAction,
  onGenerate,
  generating,
  error,
  result,
  pushedTo,
  onOpenPullRequest,
}: GitCommitBoxProps) {
  const tokens = useTokens();
  const [menuOpen, setMenuOpen] = useState(false);
  const [focusedControl, setFocusedControl] = useState<string | null>(null);
  const trimmed = message.trim();
  const effectiveAction: CommitAction = canPush ? action : "commit";
  const busy = committing || pushing;
  const blockedReason =
    stagedCount === 0
      ? "nothing staged to commit"
      : trimmed.length === 0
        ? "a commit message is required"
        : null;
  const disabled = blockedReason != null || busy;
  const filesLabel = `${stagedCount} file${stagedCount === 1 ? "" : "s"}`;
  const primaryLabel = `${COMMIT_ACTION_LABEL[effectiveAction]} ${filesLabel}`;
  const run = (next: CommitAction) => {
    setMenuOpen(false);
    if (!disabled) onCommit(next);
  };
  const generateBlocked = stagedCount === 0;

  return (
    <View style={[styles.box, { borderTopColor: tokens.border }]}>
      <View>
        <TextInput
          value={message}
          onChangeText={onChangeMessage}
          onKeyPress={(event) => {
            const native = event.nativeEvent as unknown as { key?: string; ctrlKey?: boolean; metaKey?: boolean };
            if (!isCommitChord(native)) return;
            (event as unknown as { preventDefault?: () => void }).preventDefault?.();
            run(effectiveAction);
          }}
          multiline
          numberOfLines={3}
          editable={!busy}
          placeholder="commit message  (Ctrl+Enter to commit)"
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
          onPress={generateBlocked || generating || busy ? undefined : onGenerate}
          disabled={generateBlocked || generating || busy}
          onFocus={() => setFocusedControl("generate")}
          onBlur={() => setFocusedControl(null)}
          accessibilityRole="button"
          accessibilityLabel={
            generateBlocked ? "Generate commit message unavailable, nothing staged" : "Generate commit message from the staged changes"
          }
          accessibilityState={{ disabled: generateBlocked || busy, busy: generating }}
          hitSlop={6}
          style={[
            styles.sparkle,
            {
              borderRadius: radii.radius4,
              borderColor: focusedControl === "generate" ? tokens.accent : "transparent",
              backgroundColor: tokens.bg2,
              opacity: generateBlocked ? 0.4 : 1,
            },
          ]}
        >
          {generating ? (
            <ActivityIndicator size="small" color={tokens.accent} />
          ) : (
            <Sparkles size={14} strokeWidth={1.75} color={tokens.accent} />
          )}
        </Pressable>
      </View>

      <View style={styles.split}>
        <Pressable
          onPress={disabled ? undefined : () => run(effectiveAction)}
          disabled={disabled}
          onFocus={() => setFocusedControl("primary")}
          onBlur={() => setFocusedControl(null)}
          accessibilityRole="button"
          accessibilityState={{ disabled, busy }}
          accessibilityLabel={blockedReason ? `Commit unavailable — ${blockedReason}` : primaryLabel}
          style={[
            styles.primary,
            canPush ? styles.primaryJoined : null,
            {
              // ds/Button's disabled convention (flat bg3 fill + ink4 label) at dock density —
              // its own 48px primary height is twice this row.
              backgroundColor: disabled ? tokens.bg3 : tokens.accent,
              borderColor: focusedControl === "primary" ? tokens.ink : "transparent",
            },
          ]}
        >
          <Text style={[typeScale.meta, { color: disabled ? tokens.ink4 : tokens.onAccent }]}>
            {committing ? "Committing…" : pushing ? "Pushing…" : primaryLabel}
          </Text>
        </Pressable>
        {canPush ? (
          <Pressable
            onPress={busy ? undefined : () => setMenuOpen((open) => !open)}
            disabled={busy}
            onFocus={() => setFocusedControl("caret")}
            onBlur={() => setFocusedControl(null)}
            accessibilityRole="button"
            accessibilityState={{ expanded: menuOpen, disabled: busy }}
            accessibilityLabel="Choose commit action"
            style={[
              styles.caret,
              {
                backgroundColor: disabled && blockedReason ? tokens.bg3 : tokens.accent,
                borderColor: focusedControl === "caret" ? tokens.ink : tokens.border,
              },
            ]}
          >
            <ChevronDown
              size={13}
              strokeWidth={2}
              color={disabled && blockedReason ? tokens.ink4 : tokens.onAccent}
            />
          </Pressable>
        ) : null}
      </View>
      {menuOpen ? (
        <View style={[styles.menu, { backgroundColor: tokens.bg2, borderColor: tokens.border, borderRadius: radii.radius4 }]}>
          {(["commit", "commit-push", "commit-push-pr"] as const).map((option) => (
            <MenuItem
              key={option}
              option={option}
              current={option === effectiveAction}
              disabled={disabled}
              onPress={() => {
                onChooseAction(option);
                run(option);
              }}
            />
          ))}
        </View>
      ) : null}

      {blockedReason && !busy ? (
        <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>{blockedReason}</Text>
      ) : null}
      {onOpenPullRequest && canPush && stagedCount === 0 && !busy ? (
        <Pressable
          onPress={onOpenPullRequest}
          accessibilityRole="button"
          accessibilityLabel="Create or view the pull request for this branch"
          hitSlop={6}
        >
          <Text style={[typeScale.monoMeta, { color: tokens.accent }]}>pull request for this branch</Text>
        </Pressable>
      ) : null}
      {error ? <Text style={[typeScale.monoMeta, { color: tokens.danger }]}>{error.message}</Text> : null}
      {result && !error ? (
        <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.success }]} numberOfLines={2}>
          ✓ {result.sha.slice(0, SHORT_SHA)} {result.summary}
          {pushedTo ? ` · pushed to ${pushedTo}` : ""}
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
    paddingRight: 34,
    textAlignVertical: "top",
  },
  sparkle: {
    position: "absolute",
    top: 5,
    right: 5,
    width: 24,
    height: 24,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 1,
  },
  split: { flexDirection: "row" },
  primary: {
    flex: 1,
    height: 28,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radii.radius4,
    borderWidth: 1,
  },
  primaryJoined: { borderTopRightRadius: 0, borderBottomRightRadius: 0 },
  caret: {
    width: 28,
    height: 28,
    alignItems: "center",
    justifyContent: "center",
    borderTopRightRadius: radii.radius4,
    borderBottomRightRadius: radii.radius4,
    borderWidth: 1,
    borderLeftWidth: StyleSheet.hairlineWidth,
  },
  menu: { borderWidth: 1, overflow: "hidden" },
  menuItem: {
    minHeight: 30,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: space.space8,
  },
  branch: { fontFamily: monoFamily.regular },
});
