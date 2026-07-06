// DESIGN_SYSTEM.md §6 Containers — ConfirmDialog: centered <=360pt card.
// Destructive variant is 2-step: primary (safe) action is Cancel, the danger
// action requires a 400ms press-and-hold with a filling background.
//
// The two action buttons are built inline rather than composed from
// `ds/Button` (owned by the parallel T1.1 task): the danger action's
// press-and-hold fill is a bespoke interaction outside Button's D/P/F/L/X
// state machine, so keeping this file self-contained avoids a fragile
// cross-task prop-shape dependency.
import React, { useCallback, useRef } from "react";
import { Modal, Pressable, StyleSheet, Text, View } from "react-native";
import Animated, {
  cancelAnimation,
  runOnJS,
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withTiming,
} from "react-native-reanimated";

import { haptics } from "../../lib/haptics";
import { useTokens } from "../../theme/ThemeProvider";
import { durations, easings } from "../../theme/motion";
import { radii, space, tapTarget } from "../../theme/tokens";
import { type } from "../../theme/typography";

export interface ConfirmDialogProps {
  visible: boolean;
  title: string;
  message?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  /** Destructive variant: Cancel is the prominent action, Confirm requires a press-and-hold. */
  destructive?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

const HOLD_MS = 400;

export function ConfirmDialog({
  visible,
  title,
  message,
  confirmLabel = "Confirm",
  cancelLabel = "Cancel",
  destructive = false,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const tokens = useTokens();
  const reduced = useReducedMotion();
  const fill = useSharedValue(0);
  const firedRef = useRef(false);

  const complete = useCallback(() => {
    if (firedRef.current) return;
    firedRef.current = true;
    haptics.deny();
    onConfirm();
  }, [onConfirm]);

  const onHoldIn = () => {
    firedRef.current = false;
    if (reduced) {
      fill.value = 1;
      complete();
      return;
    }
    fill.value = withTiming(1, { duration: HOLD_MS, easing: easings.linear }, (finished) => {
      if (finished) runOnJS(complete)();
    });
  };

  const onHoldOut = () => {
    if (firedRef.current) return;
    cancelAnimation(fill);
    fill.value = withTiming(0, { duration: durations.fast, easing: easings.standard });
  };

  const fillStyle = useAnimatedStyle(() => ({ width: `${fill.value * 100}%` }));

  if (!visible) return null;

  return (
    <Modal
      visible={visible}
      transparent
      animationType={reduced ? "none" : "fade"}
      onRequestClose={onCancel}
      statusBarTranslucent
    >
      <View style={[styles.scrim, { backgroundColor: tokens.overlayScrim }]}>
        <Pressable
          style={StyleSheet.absoluteFill}
          onPress={onCancel}
          accessibilityRole="button"
          accessibilityLabel="Dismiss"
        />
        <View
          style={[styles.card, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}
          accessibilityViewIsModal
          accessibilityRole="none"
        >
          <Text style={[type.heading, { color: tokens.ink }]}>{title}</Text>
          {message ? <Text style={[type.sub, styles.message, { color: tokens.ink2 }]}>{message}</Text> : null}

          <View style={styles.actions}>
            <Pressable
              onPress={onCancel}
              accessibilityRole="button"
              accessibilityLabel={cancelLabel}
              style={[styles.button, { backgroundColor: tokens.accent }]}
            >
              <Text style={[type.bodyBold, { color: tokens.onAccent }]}>{cancelLabel}</Text>
            </Pressable>

            {destructive ? (
              <Pressable
                onPressIn={onHoldIn}
                onPressOut={onHoldOut}
                accessibilityRole="button"
                accessibilityLabel={confirmLabel}
                accessibilityHint="Press and hold to confirm"
                accessibilityActions={[{ name: "activate", label: confirmLabel }]}
                onAccessibilityAction={(e) => {
                  if (e.nativeEvent.actionName === "activate") complete();
                }}
                style={[styles.button, styles.holdButton, { borderColor: tokens.danger }]}
              >
                <Animated.View
                  style={[styles.holdFill, { backgroundColor: tokens.dangerBg, pointerEvents: "none" }, fillStyle]}
                />
                <Text style={[type.bodyBold, { color: tokens.danger }]}>{confirmLabel}</Text>
              </Pressable>
            ) : (
              <Pressable
                onPress={onConfirm}
                accessibilityRole="button"
                accessibilityLabel={confirmLabel}
                style={[styles.button, styles.ghostButton, { borderColor: tokens.border }]}
              >
                <Text style={[type.bodyBold, { color: tokens.ink }]}>{confirmLabel}</Text>
              </Pressable>
            )}
          </View>
        </View>
      </View>
    </Modal>
  );
}

const styles = StyleSheet.create({
  scrim: { flex: 1, alignItems: "center", justifyContent: "center", padding: space.space24 },
  card: {
    width: "100%",
    maxWidth: 360,
    borderRadius: radii.radius16,
    borderWidth: StyleSheet.hairlineWidth,
    padding: space.space20,
    gap: space.space8,
  },
  message: { marginTop: space.space4 },
  actions: { flexDirection: "row", gap: space.space12, marginTop: space.space16 },
  button: {
    flex: 1,
    minHeight: tapTarget,
    borderRadius: radii.radius8,
    alignItems: "center",
    justifyContent: "center",
    overflow: "hidden",
  },
  ghostButton: { borderWidth: StyleSheet.hairlineWidth },
  holdButton: { borderWidth: StyleSheet.hairlineWidth },
  holdFill: { position: "absolute", left: 0, top: 0, bottom: 0 },
});
