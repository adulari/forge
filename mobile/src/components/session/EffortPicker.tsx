// Hearth "Effort" — the heat-ramp control (native handoff pattern 4): a 6px thermal
// gradient track with an ember thumb that snaps to the selected detent, mono uppercase
// detent labels, a per-detent meaning + cost/latency line, and a session-default reset.
// Tap/press to pick a detent (no drag) — robust across native + web — then commit.
import { Flame } from "lucide-react-native";
import React, { useEffect, useState } from "react";
import { LinearGradient } from "expo-linear-gradient";
import { Modal, Platform, Pressable, StyleSheet, Text, View } from "react-native";

import type { RemoteInput } from "../../lib/ws";
import { useTheme, useTokens } from "../../theme/ThemeProvider";
import { depthDark, depthLight, radii, space, tapTarget, type ColorTokens } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";
import { Button } from "../ds/Button";
import { Chip } from "../ds/Chip";
import { Sheet } from "../ds/Sheet";
import { useToast } from "../ds/ToastHost";

export const EFFORT_LEVELS = ["low", "medium", "high", "xhigh", "whitehot"] as const;
export type EffortLevel = (typeof EFFORT_LEVELS)[number];

// The ramp includes a leading "default" detent (session default / let the mesh pick) that
// is NOT an EffortLevel — selecting it resets rather than pinning a level.
type Detent = "default" | EffortLevel;
const DETENTS: readonly Detent[] = ["default", ...EFFORT_LEVELS] as const;

interface DetentMeta {
  meaning: string;
  cost: string;
  /** true only for whitehot — its label always paints in warnBgInk (the white-hot ink). */
  whitehot?: boolean;
}

const DETENT_META: Record<Detent, DetentMeta> = {
  default: { meaning: "let the mesh pick per task", cost: "~$0.02/turn" },
  low: { meaning: "quick, shallow passes", cost: "~$0.03/turn" },
  medium: { meaning: "brief thinking, fast replies", cost: "~$0.05/turn" },
  high: { meaning: "extended thinking on every turn", cost: "~$0.18/turn · slower" },
  xhigh: { meaning: "maximum single-model reasoning", cost: "~$0.60/turn" },
  whitehot: { meaning: "council of frontier models argue it out", cost: "~$2.40/turn · minutes", whitehot: true },
};

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

function labelColor(detent: Detent, selected: boolean, tokens: ColorTokens): string {
  if (DETENT_META[detent].whitehot) return tokens.warnBgInk;
  if (selected) return tokens.accent;
  return tokens.ink4;
}

export function EffortPicker({ effort, send, visible: controlledVisible, onClose, showTrigger = true }: EffortPickerProps) {
  const tokens = useTokens();
  const toast = useToast();
  const { isCompact } = useBreakpoint();
  const [localVisible, setLocalVisible] = useState(false);
  const [pending, setPending] = useState<EffortLevel | null>(null);
  const visible = controlledVisible ?? localVisible;

  // The currently active effort (server truth, or optimistic `pending`).
  const active: Detent = pending ?? (isEffortLevel(effort) ? effort : "default");
  // The detent the user is previewing inside the sheet before committing.
  const [preview, setPreview] = useState<Detent>(active);

  const close = () => {
    setLocalVisible(false);
    onClose?.();
  };

  useEffect(() => {
    if (pending != null && effort === pending) setPending(null);
  }, [effort, pending]);

  // Re-seed the preview from the active effort whenever the sheet opens.
  useEffect(() => {
    if (visible) setPreview(active);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible]);

  const commit = (detent: Detent) => {
    close();
    const command = detent === "default" ? "/effort" : `/effort ${detent}`;
    if (detent !== "default") setPending(detent);
    if (!send({ kind: "prompt", text: command })) {
      setPending(null);
      toast.show("not sent — reconnect and try again", { tone: "danger" });
    }
  };

  const activeIndex = DETENTS.indexOf(preview);
  const thumbFraction = DETENTS.length > 1 ? activeIndex / (DETENTS.length - 1) : 0;
  const whitehotChip = active === "whitehot";

  // Effort presents as a bottom sheet on compact and a centered ~560px popover on desktop-web
  // (README: "sheet (mobile) / popover (desktop-web)"). The ramp body is identical either way.
  const panel = (
    <View style={styles.content}>
          <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Effort</Text>
          <Text style={[typeScale.sub, styles.subtitle, { color: tokens.ink3 }]}>How hard the model thinks on each turn.</Text>

          <View style={styles.trackWrap}>
            <LinearGradient
              colors={[tokens.warnBg, tokens.ember.ember700, tokens.ember.ember600, tokens.ember.ember500, tokens.ember.ember400, tokens.warnBgInk]}
              start={{ x: 0, y: 0 }}
              end={{ x: 1, y: 0 }}
              style={styles.track}
            />
            <View style={[styles.thumb, { left: `${thumbFraction * 100}%`, backgroundColor: tokens.accent, borderColor: tokens.bg2, shadowColor: tokens.accent }]} pointerEvents="none" />
          </View>

          <View style={styles.detentLabels}>
            {DETENTS.map((detent) => {
              const selected = detent === preview;
              return (
                <Text key={detent} style={[styles.detentLabel, { color: labelColor(detent, selected, tokens), fontWeight: selected ? "700" : "400" }]}>
                  {detent.toUpperCase()}
                </Text>
              );
            })}
          </View>

          <View style={styles.options} accessibilityRole="radiogroup" accessibilityLabel="Reasoning effort choices">
            {DETENTS.map((detent, index) => {
              const selected = detent === preview;
              const meta = DETENT_META[detent];
              const labelInk = meta.whitehot ? tokens.warnBgInk : selected ? tokens.accent : tokens.ink3;
              return (
                <React.Fragment key={detent}>
                  {index > 0 ? <View style={[styles.hairline, { backgroundColor: tokens.hairline }]} /> : null}
                  <Pressable
                    onPress={() => setPreview(detent)}
                    accessibilityRole="radio"
                    accessibilityState={{ checked: selected }}
                    accessibilityLabel={`${detent} — ${meta.meaning}`}
                    style={[styles.option, selected ? { backgroundColor: tokens.selection } : null]}
                  >
                    <Text style={[styles.optionKey, tabularNums, { color: labelInk, fontWeight: selected || meta.whitehot ? "700" : "400" }]} numberOfLines={1}>
                      {detent}
                    </Text>
                    <Text style={[typeScale.sub, styles.optionMeaning, { color: selected ? tokens.ink : tokens.ink2 }]} numberOfLines={1}>
                      {meta.meaning}
                    </Text>
                    <Text style={[styles.optionCost, tabularNums, { color: meta.whitehot ? tokens.warn : selected ? tokens.ink3 : tokens.ink4 }]} numberOfLines={1}>
                      {meta.cost}
                    </Text>
                  </Pressable>
                </React.Fragment>
              );
            })}
          </View>

          <View style={styles.actions}>
            <Button label={`Set effort · ${preview}`} onPress={() => commit(preview)} style={styles.setButton} accessibilityLabel={`Set effort to ${preview}`} />
            <Pressable onPress={() => commit("default")} accessibilityRole="button" accessibilityLabel="Reset to session default" style={styles.reset} hitSlop={8}>
              <Text style={[typeScale.sub, { color: tokens.ink3 }]}>Reset to session default</Text>
            </Pressable>
          </View>
        </View>
  );

  return (
    <>
      {showTrigger ? (
        <Chip
          label={`effort: ${active}`}
          selected={whitehotChip}
          icon={whitehotChip ? <Flame size={14} strokeWidth={1.75} color={tokens.accent} /> : undefined}
          onPress={() => setLocalVisible(true)}
          testID="effort-picker"
        />
      ) : null}
      {isCompact ? (
        <Sheet visible={visible} onClose={close} accessibilityLabel="Reasoning effort" snapPoints={[0.72]}>
          {panel}
        </Sheet>
      ) : (
        <EffortPopover visible={visible} onClose={close}>
          {panel}
        </EffortPopover>
      )}
    </>
  );
}

// Desktop-web treatment: a centered ~560px popover anchored near the top (desktop prototype
// "NF Desktop Effort"), rather than a bottom sheet. Scrim-press and Esc (web) dismiss it.
function EffortPopover({ visible, onClose, children }: { visible: boolean; onClose: () => void; children: React.ReactNode }) {
  const tokens = useTokens();
  const { scheme } = useTheme();
  const depth = scheme === "dark" ? depthDark : depthLight;

  useEffect(() => {
    if (!visible || Platform.OS !== "web") return;
    const handler = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [visible, onClose]);

  if (!visible) return null;

  return (
    <Modal visible transparent animationType="fade" onRequestClose={onClose} statusBarTranslucent>
      <View style={[styles.popoverScrim, { backgroundColor: tokens.overlayScrim }]}>
        <Pressable style={StyleSheet.absoluteFill} onPress={onClose} accessibilityRole="button" accessibilityLabel="Close" />
        <View
          style={[styles.popoverCard, { backgroundColor: tokens.bg2, borderColor: tokens.borderStrong }, depth.sheet]}
          accessibilityViewIsModal
        >
          {children}
        </View>
      </View>
    </Modal>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space20, paddingBottom: space.space24 },
  popoverScrim: { flex: 1, alignItems: "center", justifyContent: "flex-start", paddingTop: 120, paddingHorizontal: space.space24 },
  popoverCard: {
    width: "100%",
    maxWidth: 560,
    paddingTop: space.space20,
    borderRadius: radii.radius16,
    borderWidth: StyleSheet.hairlineWidth,
    overflow: "hidden",
  },
  subtitle: { marginTop: 2 },
  trackWrap: { marginTop: space.space20, height: 6, justifyContent: "center" },
  track: { height: 6, borderRadius: 3 },
  thumb: {
    position: "absolute",
    top: -8,
    marginLeft: -11,
    width: 22,
    height: 22,
    borderRadius: 11,
    borderWidth: 3,
    shadowOpacity: 0.5,
    shadowRadius: 14,
    shadowOffset: { width: 0, height: 0 },
    elevation: 6,
  },
  detentLabels: { flexDirection: "row", justifyContent: "space-between", marginTop: space.space12 },
  detentLabel: { fontFamily: monoFamily.regular, fontSize: 9.5, letterSpacing: 0.4 },
  options: { marginTop: space.space16 },
  hairline: { height: StyleSheet.hairlineWidth },
  option: { minHeight: 46, flexDirection: "row", alignItems: "center", gap: space.space8, marginHorizontal: -space.space20, paddingHorizontal: space.space20 },
  optionKey: { fontFamily: monoFamily.regular, fontSize: 12, width: 76 },
  optionMeaning: { flex: 1 },
  optionCost: { fontFamily: monoFamily.regular, fontSize: 11 },
  actions: { flexDirection: "row", alignItems: "center", gap: space.space12, marginTop: space.space16 },
  setButton: { flex: 1 },
  reset: { minHeight: tapTarget, justifyContent: "center" },
});
