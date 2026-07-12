// DESIGN.md "Mobile/desktop (V3)": the recording pill the Composer's input row morphs into
// while `lib/voice/` is capturing. Extracted from Composer.tsx to keep the frequent (~12Hz)
// amplitude-bar re-renders scoped to this component instead of the whole composer.
import { Check, X } from "lucide-react-native";
import Animated, { useAnimatedStyle, useReducedMotion, useSharedValue, withSequence, withTiming } from "react-native-reanimated";
import React, { useEffect, useRef, useState } from "react";
import { Platform, StyleSheet, Text, View } from "react-native";

import { haptics } from "../../lib/haptics";
import { useTranscribe } from "../../lib/queries";
import { chordHold, PUSH_TO_TALK_MIN_MS } from "../../lib/voice/chordHold";
import { voice } from "../../lib/voice/voice";
import { useThermalPulse } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space, tapTarget } from "../../theme/tokens";
import { tabularNums, type } from "../../theme/typography";
import { IconButton } from "../ds/IconButton";
import { StatusDot } from "../ds/StatusDot";

const BAR_COUNT = 24;
const BAR_MAX_HEIGHT = 22;
const SHAKE_DISTANCE = 6;
const ERROR_CLEAR_MS = 2600;

export interface VoiceRecordingPillProps {
  /** Appends the transcript to the composer draft and morphs back to the normal input row. */
  onAppend: (text: string) => void;
  /** Cancel, error auto-clear, or a settled accept — always morphs back, draft untouched. */
  onClose: () => void;
}

function formatElapsed(totalSeconds: number): string {
  const m = Math.floor(totalSeconds / 60);
  const s = totalSeconds % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

export function VoiceRecordingPill({ onAppend, onClose }: VoiceRecordingPillProps) {
  const tokens = useTokens();
  const reduced = useReducedMotion();
  const transcribe = useTranscribe();
  const [phase, setPhase] = useState<"recording" | "transcribing" | "error">("recording");
  const [levels, setLevels] = useState<number[]>(() => Array(BAR_COUNT).fill(0));
  const [elapsed, setElapsed] = useState(0);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const startedAt = useRef(0);
  const closedRef = useRef(false);

  const shakeX = useSharedValue(0);
  const shakeStyle = useAnimatedStyle(() => ({ transform: [{ translateX: shakeX.value }] }));
  const transcribingPulse = useThermalPulse(phase === "transcribing");

  const close = () => {
    if (closedRef.current) return;
    closedRef.current = true;
    onClose();
  };

  useEffect(() => {
    let cancelled = false;
    startedAt.current = Date.now();
    voice
      .start((rms01) => {
        if (cancelled) return;
        setLevels((prev) => [...prev.slice(1), rms01]);
      })
      .then(() => haptics.select())
      .catch((err: unknown) => {
        if (cancelled) return;
        setPhase("error");
        setErrorMsg(err instanceof Error ? err.message : "couldn't start recording");
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (phase !== "recording") return;
    const id = setInterval(() => setElapsed(Math.floor((Date.now() - startedAt.current) / 1000)), 500);
    return () => clearInterval(id);
  }, [phase]);

  useEffect(() => {
    if (phase !== "error") return;
    shakeX.value = reduced
      ? 0
      : withSequence(
          withTiming(-SHAKE_DISTANCE, { duration: 60 }),
          withTiming(SHAKE_DISTANCE, { duration: 60 }),
          withTiming(-SHAKE_DISTANCE, { duration: 60 }),
          withTiming(0, { duration: 60 }),
        );
    const t = setTimeout(close, ERROR_CLEAR_MS);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase]);

  const handleCancel = () => {
    haptics.deny();
    void voice.cancel();
    close();
  };

  const handleAccept = async () => {
    haptics.allow();
    setPhase("transcribing");
    try {
      const { blobOrFile, name } = await voice.stop();
      const form = new FormData();
      // Mirrors attach.ts's native/web split: RN's `{uri,name,type}` shorthand only means
      // anything through React Native's own networking layer, while a real web `Blob` needs
      // the explicit filename as FormData's third argument.
      if (Platform.OS === "web") {
        form.append("file", blobOrFile as Blob, name);
      } else {
        form.append("file", blobOrFile as unknown as Blob);
      }
      const res = await transcribe.mutateAsync({ form });
      onAppend(res.text);
      close();
    } catch (err) {
      setPhase("error");
      setErrorMsg(err instanceof Error ? err.message : "transcription failed");
      haptics.mergeConflict();
    }
  };

  // Web/desktop: Escape cancels, Enter stops+transcribes, and Ctrl/Cmd+Shift+V (the same combo
  // Composer.tsx used to start this recording) toggles it off. Document-level, not tied to any
  // button's focus, so it works as long as the pill is on screen. Gated to `phase === "recording"`
  // — once transcribing or erroring there's nothing left to cancel or stop early.
  //
  // The same chord also does push-to-talk, auto-detected from hold duration: Composer stamps
  // `chordHold.startedAt` on the keydown that started this recording, and the keyup of ANY part
  // of the chord (V or a modifier, whichever releases first) lands here — the composer's input
  // row is already unmounted by then. Held >= PUSH_TO_TALK_MIN_MS ⇒ stop+transcribe on release;
  // shorter was a tap ⇒ recording continues in toggle mode exactly as before.
  useEffect(() => {
    if (Platform.OS !== "web") return;

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.repeat || phase !== "recording") return;
      const target = e.target;
      if (target instanceof HTMLElement && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)) {
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        handleCancel();
      } else if (e.key === "Enter") {
        e.preventDefault(); // stop it reaching anything that would submit/insert a newline
        void handleAccept();
      } else if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "v") {
        // Only a genuinely NEW tap may toggle-stop: while the starting chord is still held
        // (no keyup seen yet) its keydowns must not end the recording. `e.repeat` already
        // filters V's own auto-repeat; the `chordHold` check is the belt-and-braces for it.
        if (chordHold.startedAt != null) return;
        e.preventDefault();
        void handleAccept();
      }
    };

    const onKeyUp = (e: KeyboardEvent) => {
      if (chordHold.startedAt == null) return;
      const key = e.key.toLowerCase();
      if (key !== "v" && key !== "shift" && key !== "control" && key !== "meta") return;
      const heldMs = Date.now() - chordHold.startedAt;
      // First release of any chord part settles the gesture — clear unconditionally so later
      // keyups of the remaining chord keys (or unrelated modifier use mid-recording) are inert.
      chordHold.startedAt = null;
      if (phase !== "recording") return;
      if (heldMs >= PUSH_TO_TALK_MIN_MS) void handleAccept();
    };

    document.addEventListener("keydown", onKeyDown);
    document.addEventListener("keyup", onKeyUp);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      document.removeEventListener("keyup", onKeyUp);
    };
    // Deliberately scoped to `phase` only: it starts at "recording" and won't change again
    // until one of these handlers fires, so the closures captured here can't go stale mid-flight.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase]);

  return (
    <Animated.View style={[styles.wrap, { backgroundColor: tokens.bg3, borderRadius: radii.radius12 }, shakeStyle]}>
      {phase === "error" ? (
        <Text style={[type.meta, styles.errorText, { color: tokens.danger }]} numberOfLines={1}>
          {errorMsg ?? "something went wrong"}
        </Text>
      ) : (
        <>
          <StatusDot
            state={phase === "recording" ? "waiting" : "busy"}
            size={10}
            accessibilityLabel={phase === "recording" ? "recording" : "transcribing"}
          />
          {phase === "recording" ? (
            <View style={styles.bars}>
              {levels.map((lvl, i) => (
                <View
                  key={i}
                  style={[styles.bar, { height: Math.max(3, lvl * BAR_MAX_HEIGHT), backgroundColor: tokens.accent }]}
                />
              ))}
            </View>
          ) : (
            <Animated.Text style={[type.meta, styles.bars, transcribingPulse, { color: tokens.ink2 }]}>
              transcribing…
            </Animated.Text>
          )}
          <Text style={[type.meta, tabularNums, { color: tokens.ink3 }]}>{formatElapsed(elapsed)}</Text>
          {phase === "recording" ? (
            <>
              <IconButton
                icon={<X size={18} strokeWidth={2} color={tokens.ink2} />}
                onPress={handleCancel}
                accessibilityLabel="cancel recording"
                testID="voice-cancel"
              />
              <IconButton
                icon={<Check size={18} strokeWidth={2} color={tokens.onAccent} />}
                onPress={() => void handleAccept()}
                accessibilityLabel="stop and transcribe"
                testID="voice-accept"
                style={[styles.accept, { backgroundColor: tokens.accent }]}
              />
            </>
          ) : null}
        </>
      )}
    </Animated.View>
  );
}

const styles = StyleSheet.create({
  wrap: {
    flexDirection: "row",
    alignItems: "center",
    height: tapTarget,
    paddingHorizontal: space.space12,
    gap: space.space8,
  },
  bars: {
    flex: 1,
    flexDirection: "row",
    alignItems: "flex-end",
    justifyContent: "space-between",
    height: BAR_MAX_HEIGHT,
  },
  bar: { width: 2, borderRadius: 1 },
  errorText: { flex: 1 },
  accept: { borderRadius: radii.radiusPill, width: tapTarget, height: tapTarget },
});
