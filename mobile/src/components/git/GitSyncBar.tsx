// Fetch / pull / push for the dock's branch, with the ahead/behind counts git reports against the
// upstream. Never forces anything: pull is fast-forward-only unless the person picks "merge
// instead" after git refuses, and push never rewrites remote history. Results land in one quiet
// line under the buttons; a failure keeps git's own words.
import { ArrowDown, ArrowUp, RefreshCw } from "lucide-react-native";
import React, { useState } from "react";
import { ActivityIndicator, Pressable, StyleSheet, Text, View } from "react-native";

import { aheadBehindLabel, isNotFastForward } from "./gitReviewSupport";
import { type GitStatusResponse } from "../../lib/api";
import { useFetchGit, usePullGit, usePushGit } from "../../lib/gitActions";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { tabularNums, type as typeScale } from "../../theme/typography";

interface Feedback {
  tone: "ok" | "error";
  text: string;
}

function SyncButton({
  label,
  detail,
  busy,
  disabled,
  icon,
  onPress,
}: {
  label: string;
  detail?: string;
  busy: boolean;
  disabled: boolean;
  icon: React.ReactNode;
  onPress: () => void;
}) {
  const tokens = useTokens();
  const [focused, setFocused] = useState(false);
  const [hovered, setHovered] = useState(false);
  return (
    <Pressable
      onHoverIn={() => setHovered(true)}
      onHoverOut={() => setHovered(false)}
      onPress={disabled || busy ? undefined : onPress}
      disabled={disabled || busy}
      onFocus={() => setFocused(true)}
      onBlur={() => setFocused(false)}
      accessibilityRole="button"
      accessibilityLabel={detail ? `${label}, ${detail}` : label}
      accessibilityState={{ disabled, busy }}
      style={[
        styles.button,
        {
          borderRadius: radii.radius4,
          borderColor: focused ? tokens.accent : tokens.border,
          backgroundColor: hovered && !disabled ? tokens.bg3 : "transparent",
          opacity: disabled ? 0.45 : 1,
        },
      ]}
    >
      {busy ? <ActivityIndicator size="small" color={tokens.accent} /> : detail ? null : icon}
      <Text style={[typeScale.monoMeta, { color: tokens.ink2 }]} numberOfLines={1}>
        {label}
      </Text>
      {detail ? <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.accent }]}>{detail}</Text> : null}
    </Pressable>
  );
}

export function GitSyncBar({ sessionId, status }: { sessionId: string; status: GitStatusResponse }) {
  const tokens = useTokens();
  const fetchGit = useFetchGit();
  const pull = usePullGit();
  const push = usePushGit();
  const [feedback, setFeedback] = useState<Feedback | null>(null);
  const [offerMerge, setOfferMerge] = useState(false);
  const ahead = status.ahead ?? 0;
  const behind = status.behind ?? 0;
  const hasRemote = status.has_remote !== false;
  const busy = fetchGit.isPending || pull.isPending || push.isPending;
  const detached = !status.branch || status.branch === "HEAD";
  const unfinished = status.operation ?? null;
  const conflictCount = status.conflicted?.length ?? 0;

  const run = async (
    action: () => Promise<{ output: string; remote: string | null }>,
    done: (output: string, remote: string | null) => string,
  ) => {
    setFeedback(null);
    setOfferMerge(false);
    try {
      const result = await action();
      setFeedback({ tone: "ok", text: done(result.output, result.remote) });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setFeedback({ tone: "error", text: message });
      setOfferMerge(isNotFastForward(message));
    }
  };

  const doFetch = () =>
    run(() => fetchGit.mutateAsync({ session: sessionId }), () => "Fetched.");
  const doPull = (strategy?: "merge") =>
    run(
      () => pull.mutateAsync({ session: sessionId, strategy }),
      (output) => (/already up to date/i.test(output) ? "Already up to date." : "Pulled."),
    );
  const doPush = () =>
    run(
      () => push.mutateAsync({ session: sessionId }),
      (_output, remote) => `Pushed ${status.branch}${remote ? ` to ${remote}` : ""}.`,
    );

  return (
    <View style={[styles.root, { borderBottomColor: tokens.border }]}>
      <View style={styles.row}>
        <SyncButton
          label="Fetch"
          busy={fetchGit.isPending}
          disabled={!hasRemote || busy}
          icon={<RefreshCw size={12} strokeWidth={1.75} color={tokens.ink3} />}
          onPress={doFetch}
        />
        <SyncButton
          label="Pull"
          detail={behind > 0 ? `↓${behind}` : undefined}
          busy={pull.isPending}
          disabled={!hasRemote || detached || busy || unfinished != null}
          icon={<ArrowDown size={12} strokeWidth={1.75} color={tokens.ink3} />}
          onPress={() => doPull()}
        />
        <SyncButton
          label="Push"
          detail={ahead > 0 ? `↑${ahead}` : undefined}
          busy={push.isPending}
          disabled={!hasRemote || detached || busy || unfinished != null}
          icon={<ArrowUp size={12} strokeWidth={1.75} color={tokens.ink3} />}
          onPress={doPush}
        />
      </View>
      {!hasRemote ? (
        <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>no remote configured</Text>
      ) : status.upstream ? (
        <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]} numberOfLines={1}>
          {status.upstream}
          {aheadBehindLabel(ahead, behind) ? ` · ${aheadBehindLabel(ahead, behind)}` : " · up to date"}
        </Text>
      ) : (
        <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>no upstream yet — push sets one</Text>
      )}
      {unfinished ? (
        <Text style={[typeScale.monoMeta, { color: tokens.warn }]}>
          {unfinished} in progress
          {conflictCount > 0
            ? " — resolve the conflicts to continue"
            : unfinished === "merge"
              ? " — commit to finish it"
              : ""}
        </Text>
      ) : null}
      {feedback ? (
        <Text
          accessibilityLiveRegion="polite"
          style={[typeScale.monoMeta, { color: feedback.tone === "ok" ? tokens.success : tokens.danger }]}
        >
          {feedback.tone === "ok" ? "✓ " : ""}
          {feedback.text}
        </Text>
      ) : null}
      {offerMerge ? (
        <Pressable
          onPress={() => doPull("merge")}
          accessibilityRole="button"
          accessibilityLabel="Pull again, merging instead of fast-forwarding"
          style={styles.mergeLink}
        >
          <Text style={[typeScale.monoMeta, { color: tokens.accent }]}>pull with a merge instead</Text>
        </Pressable>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  root: { gap: 5, paddingHorizontal: 10, paddingVertical: space.space8, borderBottomWidth: StyleSheet.hairlineWidth },
  row: { flexDirection: "row", gap: 6 },
  button: {
    flex: 1,
    minHeight: 26,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 5,
    paddingHorizontal: 6,
    borderWidth: 1,
  },
  mergeLink: { alignSelf: "flex-start", paddingVertical: 2 },
});
