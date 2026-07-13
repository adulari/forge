import { Flame } from "lucide-react-native";
import React, { useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import type { RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Chip } from "../ds/Chip";
import { Sheet } from "../ds/Sheet";
import { useToast } from "../ds/ToastHost";

export const EFFORT_LEVELS = ["low", "medium", "high", "xhigh", "whitehot"] as const;
export type EffortLevel = (typeof EFFORT_LEVELS)[number];

export interface EffortPickerProps {
  effort?: string | null;
  send: (input: RemoteInput) => boolean;
  visible?: boolean;
  onClose?: () => void;
  showTrigger?: boolean;
}

function isEffortLevel(value: string | null | undefined): value is EffortLevel {
  return value != null && EFFORT_LEVELS.includes(value as EffortLevel);
}

export function EffortPicker({ effort, send, visible: controlledVisible, onClose, showTrigger = true }: EffortPickerProps) {
  const tokens = useTokens();
  const toast = useToast();
  const [localVisible, setLocalVisible] = useState(false);
  const [pending, setPending] = useState<EffortLevel | null>(null);
  const visible = controlledVisible ?? localVisible;
  const close = () => {
    setLocalVisible(false);
    onClose?.();
  };
  const current = pending ?? (isEffortLevel(effort) ? effort : "medium");

  useEffect(() => {
    if (pending != null && effort === pending) setPending(null);
  }, [effort, pending]);

  const select = (level: EffortLevel) => {
    close();
    setPending(level);
    if (!send({ kind: "prompt", text: `/effort ${level}` })) {
      setPending(null);
      toast.show("not sent — reconnect and try again", { tone: "danger" });
    }
  };

  const whitehot = current === "whitehot";
  return (
    <>
      {showTrigger ? <Chip label={`effort: ${current}`} selected={whitehot} icon={whitehot ? <Flame size={14} strokeWidth={1.75} color={tokens.accent} /> : undefined} onPress={() => setLocalVisible(true)} testID="effort-picker" /> : null}
      <Sheet visible={visible} onClose={close} accessibilityLabel="Reasoning effort" snapPoints={[0.65]}>
        <View style={styles.content} accessibilityRole="radiogroup" accessibilityLabel="Reasoning effort choices">
          <Text style={[typeScale.heading, { color: tokens.ink }]}>Reasoning effort</Text>
          <Text style={[typeScale.sub, { color: tokens.ink2 }]}>Choose how intensely Forge reasons for this session.</Text>
          <View style={styles.options}>
            {EFFORT_LEVELS.map((level) => {
              const selected = level === current;
              const isWhitehot = level === "whitehot";
              return <Pressable key={level} onPress={() => select(level)} accessibilityRole="radio" accessibilityState={{ checked: selected }} accessibilityLabel={level} style={[styles.option, { backgroundColor: selected ? tokens.selection : tokens.bg2, borderColor: selected || isWhitehot ? tokens.accent : tokens.border }]}>{isWhitehot ? <Flame size={16} strokeWidth={1.75} color={tokens.accent} /> : null}<Text style={[typeScale.bodyBold, styles.optionLabel, { color: isWhitehot ? tokens.accent : tokens.ink }]}>{level}</Text>{selected ? <Text style={[typeScale.meta, { color: tokens.ink3 }]}>current</Text> : null}</Pressable>;
            })}
          </View>
        </View>
      </Sheet>
    </>
  );
}

const styles = StyleSheet.create({ content: { paddingHorizontal: space.space16, paddingBottom: space.space16, gap: space.space8 }, options: { gap: space.space8, paddingTop: space.space8 }, option: { minHeight: 44, flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space12, borderRadius: radii.radius8, borderWidth: StyleSheet.hairlineWidth }, optionLabel: { flex: 1 } });
