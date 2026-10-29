// The running turn at the newest end of the chat: the reply as it streams, and one compact line
// saying what the turn is doing while no text is coming in ("Thinking · 12s", "Run cargo test ·
// 4s"). It replaced a bare "thinking…" beside the pull-to-refresh spinner, which is absolutely
// positioned and rendered as a large ember arc floating over the composer's chips.
import React, { useEffect, useMemo, useRef, useState } from "react";
import { StyleSheet, Text, View } from "react-native";
import Animated, {
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withRepeat,
  withSequence,
  withTiming,
} from "react-native-reanimated";

import { splitStreamedText } from "../../lib/streamCap";
import { summarizeToolLine } from "../../lib/toolRows";
import { easings, useEmberdot } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { tabularNums, type as typeScale } from "../../theme/typography";
import { Markdown } from "./Markdown";
import { useChatProseStyle } from "./proseStyle";

const CARET_SIZE = 7;
const CARET_PULSE_MS = 1000;
const CARET_MIN_OPACITY = 0.4;

/** The daemon's turn phases (crates/forge-tui/src/app.rs `TurnPhase::label`) in reader words. */
const ACTIVITY_LABELS: Record<string, string> = {
  "preparing turn": "Preparing",
  "waiting for model": "Waiting for the model",
  "receiving provider stream": "Receiving",
  "model reasoning": "Thinking",
  "streaming response": "Writing",
  "running tool": "Running a tool",
  "processing tool result": "Reading the result",
  "auxiliary model work": "Working",
  "recovering provider": "Retrying the model",
  "coordinating work": "Coordinating agents",
  "compacting context": "Compacting context",
  "finalizing turn": "Finishing",
};

export function activityLabel(activity: string | undefined, toolLine: string | null): string {
  if (toolLine && (!activity || activity === "running tool")) return summarizeToolLine(toolLine);
  if (activity) return ACTIVITY_LABELS[activity] ?? activity[0].toUpperCase() + activity.slice(1);
  return "Thinking";
}

export function formatElapsed(elapsedMs: number): string {
  const totalSeconds = Math.max(0, Math.floor(elapsedMs / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return minutes > 0 ? `${minutes}m ${seconds.toString().padStart(2, "0")}s` : `${seconds}s`;
}

/** Elapsed time since `startedAt` (Unix ms; 0 = unknown, counts from mount), ticking each second. */
function useElapsed(startedAt: number): number {
  const [mountedAt] = useState(() => Date.now());
  const [now, setNow] = useState(() => Date.now());
  // A host clock ahead of this device would read as a negative age; never start in the future.
  const origin = startedAt > 0 ? Math.min(startedAt, now) : mountedAt;
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  return now - origin;
}

export interface TurnActivityProps {
  activity?: string;
  /** The newest live tool line (`↳ shell {...}`), when a tool is running. */
  toolLine: string | null;
  startedAt: number;
}

function TurnActivityImpl({ activity, toolLine, startedAt }: TurnActivityProps) {
  const tokens = useTokens();
  const { dotStyle } = useEmberdot("busy");
  const elapsed = useElapsed(startedAt);
  const label = activityLabel(activity, toolLine);
  return (
    <View style={styles.activity} accessibilityRole="text" accessibilityLabel={`${label}, ${formatElapsed(elapsed)}`} accessibilityLiveRegion="polite">
      <Animated.View style={[styles.activityDot, { backgroundColor: tokens.accent }, dotStyle]} />
      <Text style={[typeScale.sub, styles.activityLabel, { color: tokens.ink2 }]} numberOfLines={1}>
        {label}
      </Text>
      <Text style={[typeScale.meta, tabularNums, { color: tokens.ink4 }]}>{formatElapsed(elapsed)}</Text>
    </View>
  );
}

export const TurnActivity = React.memo(TurnActivityImpl);

/** At most one committed render per frame while streaming; reduce-motion commits every update. */
function useFrameText(text: string, reduced: boolean): string {
  const [displayText, setDisplayText] = useState(text);
  const latestRef = useRef(text);
  const rafRef = useRef<number | null>(null);
  useEffect(() => {
    latestRef.current = text;
    if (reduced) {
      setDisplayText(text);
      return;
    }
    if (rafRef.current == null) {
      rafRef.current = requestAnimationFrame(() => {
        rafRef.current = null;
        setDisplayText(latestRef.current);
      });
    }
  }, [text, reduced]);
  useEffect(
    () => () => {
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
    },
    [],
  );
  return displayText;
}

/** The reply as it streams, through the same Markdown the finished row uses, so finalizing is a
 * visual no-op. Markdown memoizes finished blocks, so a token re-renders only the open block. */
export function StreamingAnswer({ text, streaming }: { text: string; streaming: boolean }) {
  const tokens = useTokens();
  const reduced = useReducedMotion();
  const prose = useChatProseStyle();
  const displayText = useFrameText(text, reduced);
  const { head, tail } = useMemo(() => splitStreamedText(displayText, streaming), [displayText, streaming]);

  const caretOpacity = useSharedValue(1);
  useEffect(() => {
    if (reduced || !streaming) {
      caretOpacity.value = 1;
      return;
    }
    caretOpacity.value = withRepeat(
      withSequence(
        withTiming(CARET_MIN_OPACITY, { duration: CARET_PULSE_MS / 2, easing: easings.standard }),
        withTiming(1, { duration: CARET_PULSE_MS / 2, easing: easings.standard }),
      ),
      -1,
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [streaming, reduced]);
  const caretStyle = useAnimatedStyle(() => ({ opacity: caretOpacity.value }));

  return (
    <View style={styles.answerRow}>
      <View style={styles.answerText}>
        {head ? (
          <Text selectable style={[typeScale.body, prose, styles.olderText, { color: tokens.ink }]}>
            {head}
          </Text>
        ) : null}
        <Markdown content={tail} style={prose} />
      </View>
      {streaming ? (
        <Animated.View
          accessibilityElementsHidden
          importantForAccessibility="no-hide-descendants"
          style={[styles.caret, { backgroundColor: tokens.accent }, caretStyle]}
        />
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  activity: { flexDirection: "row", alignItems: "center", gap: space.space8, minHeight: 28 },
  activityDot: { width: 6, height: 6, borderRadius: 3 },
  activityLabel: { flexShrink: 1, minWidth: 0 },
  answerRow: { flexDirection: "row", alignItems: "flex-end", flexWrap: "wrap" },
  answerText: { flexShrink: 1, minWidth: 0 },
  olderText: { marginBottom: space.space12 },
  caret: { width: CARET_SIZE, height: CARET_SIZE, borderRadius: CARET_SIZE / 2, marginLeft: 4, marginBottom: 8 },
});
