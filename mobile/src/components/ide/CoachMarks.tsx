// Three one-time pointers for a first visit: the queue, the file actions, the agent. Not a modal
// tour: no scrim, nothing blocks the workbench, Escape or "Skip" ends it for good.
import React, { useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import Animated, { FadeIn, FadeOut } from "react-native-reanimated";

import { markIdeCoachSeen, useIdeModePreferences } from "../../lib/ideModePrefs";
import { useOverlayEscape } from "../../lib/useOverlayEscape";
import { durations } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { sansFamilyFor } from "../../theme/typography";
import { Button } from "../ds/Button";

interface Step {
  title: string;
  body: string;
  place: "rail" | "editor" | "agent";
}

const STEPS: readonly Step[] = [
  { place: "rail", title: "Start with Changes", body: "Files are ordered by what needs you first. Tick a file once you have looked at it." },
  { place: "editor", title: "Review in the diff", body: "Accept or revert each hunk, or comment on a line. Keys: j k move, a accept, x revert, c comment." },
  { place: "agent", title: "Send it back", body: "Comments pile up as one review. Send review hands them to the agent together; approvals show up above the box." },
];

export function CoachMarks({ railWidth, agentWidth }: { railWidth: number; agentWidth: number }) {
  const tokens = useTokens();
  const { loaded, coachSeen } = useIdeModePreferences();
  const [index, setIndex] = useState(0);
  const [open, setOpen] = useState(false);
  // Wait a beat so the workbench paints first and the card does not compete with it.
  useEffect(() => {
    if (!loaded || coachSeen) return;
    const timer = setTimeout(() => setOpen(true), 700);
    return () => clearTimeout(timer);
  }, [loaded, coachSeen]);
  const visible = open && !coachSeen;
  useOverlayEscape(visible, markIdeCoachSeen);
  if (!visible) return null;
  const step = STEPS[index];
  const last = index === STEPS.length - 1;
  const place =
    step.place === "rail" ? { left: railWidth + space.space12, top: 150 } : step.place === "editor" ? { left: railWidth + space.space24, top: 52 } : { right: agentWidth + space.space12, bottom: 96 };
  return (
    <Animated.View
      entering={FadeIn.duration(durations.base)}
      exiting={FadeOut.duration(durations.fast)}
      style={[styles.card, place, { backgroundColor: tokens.bg2, borderColor: tokens.borderStrong }]}
      accessibilityRole="alert"
    >
      <Text style={[styles.step, { color: tokens.ink3 }]}>{`${index + 1} of ${STEPS.length}`}</Text>
      <Text style={[styles.title, { color: tokens.ink }]}>{step.title}</Text>
      <Text style={[styles.body, { color: tokens.ink2 }]}>{step.body}</Text>
      <View style={styles.actions}>
        <Pressable onPress={markIdeCoachSeen} accessibilityRole="button" hitSlop={6}>
          <Text style={[styles.skip, { color: tokens.ink3 }]}>Skip</Text>
        </Pressable>
        <Button label={last ? "Got it" : "Next"} variant="secondary" onPress={() => (last ? markIdeCoachSeen() : setIndex(index + 1))} style={styles.next} />
      </View>
    </Animated.View>
  );
}

const styles = StyleSheet.create({
  card: { position: "absolute", width: 264, zIndex: 50, padding: space.space16, gap: 6, borderWidth: 1, borderRadius: radii.radius8 },
  step: { fontFamily: sansFamilyFor(600), fontSize: 10, letterSpacing: 0.8, textTransform: "uppercase" },
  title: { fontFamily: sansFamilyFor(600), fontSize: 14, letterSpacing: -0.1 },
  body: { fontFamily: sansFamilyFor(400), fontSize: 12.5, lineHeight: 18 },
  actions: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", marginTop: space.space8 },
  skip: { fontFamily: sansFamilyFor(500), fontSize: 12 },
  next: { height: 28, paddingHorizontal: space.space16 },
});
