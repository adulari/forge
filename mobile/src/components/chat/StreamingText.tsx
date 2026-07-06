// DESIGN_SYSTEM.md §5.2 "Kindle" (streaming): text updates are rAF-coalesced (<=1 committed
// render per frame), a 7px ember caret dot pulses opacity 1->0.4 @1s while streaming, and on
// finalize the block cross-fades into its settled state (`base` duration). Reduce-motion: no
// caret pulse, instant text (no rAF coalescing delay either — every update commits immediately).
import React, { useEffect, useRef, useState } from "react";
import { StyleSheet, Text, type StyleProp, type TextStyle } from "react-native";
import Animated, {
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withRepeat,
  withSequence,
  withTiming,
} from "react-native-reanimated";

import { durations, easings } from "../../theme/motion";
import { useTheme } from "../../theme/ThemeProvider";
import { type } from "../../theme/typography";

const CARET_SIZE = 7;
const CARET_PULSE_MS = 1000;
const CARET_MIN_OPACITY = 0.4;

export interface StreamingTextProps {
  /** Full in-flight text (re-sent each snapshot, per the WS protocol — not a delta). */
  text: string;
  /** Whether this block is still receiving updates. */
  streaming: boolean;
  style?: StyleProp<TextStyle>;
}

export function StreamingText({ text, streaming, style }: StreamingTextProps) {
  const { tokens } = useTheme();
  const reduced = useReducedMotion();

  // Kindle: coalesce rapid `text` prop updates to at most one committed render per frame.
  // Reduce-motion skips the batching window entirely — every update commits instantly.
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

  // Caret pulse while streaming.
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

  // Cross-fade the block into its finalized state: a brief settle dip the instant
  // `streaming` flips false, skipped entirely under reduce-motion.
  const blockOpacity = useSharedValue(1);
  const wasStreaming = useRef(streaming);
  useEffect(() => {
    if (wasStreaming.current && !streaming) {
      blockOpacity.value = reduced
        ? 1
        : withSequence(
            withTiming(0.5, { duration: durations.base / 2, easing: easings.standard }),
            withTiming(1, { duration: durations.base / 2, easing: easings.standard }),
          );
    }
    wasStreaming.current = streaming;
  }, [streaming, reduced, blockOpacity]);
  const blockStyle = useAnimatedStyle(() => ({ opacity: blockOpacity.value }));

  return (
    <Animated.View style={[styles.row, blockStyle]}>
      <Text style={[type.body, { color: tokens.ink }, style]}>{displayText}</Text>
      {streaming ? (
        <Animated.View
          accessibilityElementsHidden
          importantForAccessibility="no-hide-descendants"
          style={[styles.caret, { backgroundColor: tokens.accent }, caretStyle]}
        />
      ) : null}
    </Animated.View>
  );
}

const styles = StyleSheet.create({
  row: {
    flexDirection: "row",
    alignItems: "flex-end",
    flexWrap: "wrap",
  },
  caret: {
    width: CARET_SIZE,
    height: CARET_SIZE,
    borderRadius: CARET_SIZE / 2,
    marginLeft: 4,
    marginBottom: 4,
  },
});
