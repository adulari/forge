// The prompt rail: a thin outline of every user prompt down the right edge of a long transcript.
// Hover a tick to read the prompt, press it to jump there (loading the window around it when it
// was never paged in). Pointer-only by design; the keyboard has Find and the response stepper.
import React, { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { promptPreview } from "../../lib/stickyPrompt";
import type { RailTick } from "../../lib/promptRail";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { tabularNums, type as typeScale } from "../../theme/typography";

export interface PromptRailProps {
  ticks: readonly RailTick[];
  /** Seq of the prompt the view is currently on, if any. */
  activeSeq: number | null;
  onSelect: (seq: number) => void;
}

function sentAt(epochSeconds: number): string {
  if (!epochSeconds) return "";
  return new Date(epochSeconds * 1000).toLocaleString([], { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

export function PromptRail({ ticks, activeSeq, onSelect }: PromptRailProps) {
  const tokens = useTokens();
  const [hovered, setHovered] = useState<RailTick | null>(null);
  return (
    <View style={styles.rail} pointerEvents="box-none" testID="prompt-rail" accessibilityLabel="Prompt outline">
      {ticks.map((tick) => {
        const active = tick.seq === activeSeq;
        const hot = hovered?.seq === tick.seq;
        return (
          <Pressable
            key={tick.seq}
            onPress={() => onSelect(tick.seq)}
            onHoverIn={() => setHovered(tick)}
            onHoverOut={() => setHovered((current) => (current?.seq === tick.seq ? null : current))}
            onFocus={() => setHovered(tick)}
            onBlur={() => setHovered((current) => (current?.seq === tick.seq ? null : current))}
            accessibilityRole="button"
            accessibilityLabel={`Prompt: ${promptPreview(tick.text, 120)}`}
            style={[styles.hit, { top: `${tick.top * 100}%` }]}
          >
            <View
              style={[
                styles.tick,
                {
                  width: hot ? 14 : active ? 12 : 8,
                  backgroundColor: active || hot ? tokens.accent : tokens.ink4,
                },
              ]}
            />
          </Pressable>
        );
      })}
      {hovered ? (
        <View
          pointerEvents="none"
          style={[
            styles.preview,
            { top: `${hovered.top * 100}%`, backgroundColor: tokens.bg3, borderColor: tokens.borderStrong },
          ]}
        >
          <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3 }]}>{sentAt(hovered.at)}</Text>
          <Text style={[typeScale.sub, { color: tokens.ink }]} numberOfLines={4}>
            {promptPreview(hovered.text, 280)}
          </Text>
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  rail: { position: "absolute", top: 44, bottom: 72, right: 2, width: 18, zIndex: 8 },
  hit: { position: "absolute", right: 0, width: 18, height: 10, marginTop: -5, alignItems: "flex-end", justifyContent: "center", paddingRight: 3 },
  tick: { height: 2, borderRadius: 1 },
  preview: {
    position: "absolute",
    right: 22,
    width: 280,
    marginTop: -14,
    gap: space.space4,
    paddingHorizontal: space.space12,
    paddingVertical: space.space8,
    borderWidth: 1,
    borderRadius: radii.radius8,
  },
});
