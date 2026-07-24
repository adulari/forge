// Machined "Effort" (Mobile/Desktop "Mesh Effort Duel" frames): a flat segmented strip —
// AUTO/LOW/MED/HIGH/XHIGH/WHITEHOT, WHITEHOT always painted in accent — replacing the old
// thermal gradient ramp (Machined retires that identity, see theme/tokens.ts). Tap a segment
// to preview it, read its meaning + cost caption below, then commit or reset to session
// default.
import { Flame } from "lucide-react-native";
import React, { useEffect, useState } from "react";
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
  label: string;
  meaning: string;
  cost: string;
  /** true only for whitehot — its segment label always paints in accent, selected or not. */
  whitehot?: boolean;
}

const DETENT_META: Record<Detent, DetentMeta> = {
  default: { label: "AUTO", meaning: "let the mesh pick per task", cost: "~$0.02/turn" },
  low: { label: "LOW", meaning: "quick, shallow passes", cost: "~$0.03/turn" },
  medium: { label: "MED", meaning: "brief thinking, fast replies", cost: "~$0.05/turn" },
  high: { label: "HIGH", meaning: "extended thinking on every turn", cost: "~$0.18/turn · slower" },
  xhigh: { label: "XHIGH", meaning: "maximum single-model reasoning", cost: "~$0.60/turn" },
  whitehot: {
    label: "WHITEHOT",
    meaning: "council of frontier models argue it out",
    cost: "~$2.40/turn · minutes",
    whitehot: true,
  },
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

function segmentLabelColor(detent: Detent, selected: boolean, tokens: ColorTokens): string {
  if (DETENT_META[detent].whitehot) return tokens.accent;
  if (selected) return tokens.accent;
  return tokens.ink3;
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

  const whitehotChip = active === "whitehot";

  // Effort presents as a bottom sheet on compact and a centered ~560px popover on desktop-web
  // (README: "sheet (mobile) / popover (desktop-web)"). The segmented body is identical either
  // way — Machined replaces the old thermal gradient ramp with a flat segmented strip.
  const panel = (
    <View style={styles.content}>
          <Text style={[typeScale.headingBold, { color: tokens.ink }]}>Effort</Text>
          <Text style={[typeScale.sub, styles.subtitle, { color: tokens.ink3 }]}>How hard the model thinks on each turn.</Text>

          <View
            style={[styles.segTrack, { backgroundColor: tokens.bg3, borderColor: tokens.border }]}
            accessibilityRole="radiogroup"
            accessibilityLabel="Reasoning effort choices"
          >
            {DETENTS.map((detent) => {
              const selected = detent === preview;
              const meta = DETENT_META[detent];
              return (
                <Pressable
                  key={detent}
                  onPress={() => setPreview(detent)}
                  accessibilityRole="radio"
                  accessibilityState={{ checked: selected }}
                  accessibilityLabel={`${meta.label} — ${meta.meaning}`}
                  style={[
                    styles.segItem,
                    { borderRadius: radii.radiusSegmentInner },
                    selected ? { backgroundColor: tokens.selection } : null,
                  ]}
                >
                  <Text
                    style={[styles.segLabel, tabularNums, { color: segmentLabelColor(detent, selected, tokens) }]}
                    numberOfLines={1}
                  >
                    {meta.label}
                  </Text>
                </Pressable>
              );
            })}
          </View>

          <Text style={[typeScale.monoMeta, styles.previewMeaning, { color: tokens.ink2 }]}>
            {DETENT_META[preview].meaning}
            {"  ·  "}
            <Text style={{ color: DETENT_META[preview].whitehot ? tokens.accent : tokens.ink3 }}>{DETENT_META[preview].cost}</Text>
          </Text>
          {preview !== "whitehot" ? (
            <Text style={[typeScale.meta, styles.whitehotHint, { color: tokens.ink4 }]}>
              {"whitehot — "}
              {DETENT_META.whitehot.meaning}
              {" · "}
              <Text style={{ color: tokens.accent }}>{DETENT_META.whitehot.cost}</Text>
            </Text>
          ) : null}

          <View style={styles.actions}>
            <Button label={`Set effort · ${DETENT_META[preview].label.toLowerCase()}`} onPress={() => commit(preview)} style={styles.setButton} accessibilityLabel={`Set effort to ${preview}`} />
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
  segTrack: {
    flexDirection: "row",
    marginTop: space.space20,
    padding: 3,
    minHeight: tapTarget,
    borderWidth: 1,
    borderRadius: radii.radiusSegmentOuter,
  },
  segItem: { flex: 1, alignItems: "center", justifyContent: "center" },
  segLabel: { fontFamily: monoFamily.bold, fontSize: 10, letterSpacing: 0.3 },
  previewMeaning: { marginTop: space.space12 },
  whitehotHint: { marginTop: space.space4 },
  actions: { flexDirection: "row", alignItems: "center", gap: space.space12, marginTop: space.space20 },
  setButton: { flex: 1 },
  reset: { minHeight: tapTarget, justifyContent: "center" },
});
