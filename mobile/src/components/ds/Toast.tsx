// DESIGN_SYSTEM.md §5.2 Signal — toast rises 12px + fade `base`, auto-dismiss
// 3.5s (owned by ToastHost), swipe-to-dismiss.
import React, { useEffect } from "react";
import { Pressable, StyleSheet, Text } from "react-native";
import { Gesture, GestureDetector } from "react-native-gesture-handler";
import Animated, {
  runOnJS,
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withTiming,
} from "react-native-reanimated";

import { useTokens } from "../../theme/ThemeProvider";
import { durations, easings } from "../../theme/motion";
import { radii, space } from "../../theme/tokens";
import { type } from "../../theme/typography";

export type ToastTone = "neutral" | "success" | "danger" | "warn";

export interface ToastData {
  id: string;
  message: string;
  tone?: ToastTone;
}

export interface ToastProps {
  toast: ToastData;
  onDismiss: (id: string) => void;
}

const RISE_PX = 12;
const SWIPE_DISMISS_THRESHOLD = 60;
const SWIPE_OUT_DISTANCE = 400;

export function Toast({ toast, onDismiss }: ToastProps) {
  const tokens = useTokens();
  const reduced = useReducedMotion();
  const opacity = useSharedValue(reduced ? 1 : 0);
  const translateY = useSharedValue(reduced ? 0 : RISE_PX);
  const translateX = useSharedValue(0);

  useEffect(() => {
    if (reduced) {
      opacity.value = 1;
      translateY.value = 0;
      return;
    }
    opacity.value = withTiming(1, { duration: durations.base, easing: easings.standard });
    translateY.value = withTiming(0, { duration: durations.base, easing: easings.standard });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const dismiss = () => onDismiss(toast.id);

  const swipe = Gesture.Pan()
    .onUpdate((e) => {
      translateX.value = e.translationX;
    })
    .onEnd((e) => {
      if (Math.abs(e.translationX) > SWIPE_DISMISS_THRESHOLD) {
        const direction = e.translationX > 0 ? 1 : -1;
        translateX.value = withTiming(direction * SWIPE_OUT_DISTANCE, { duration: durations.fast });
        opacity.value = withTiming(0, { duration: durations.fast }, (finished) => {
          if (finished) runOnJS(dismiss)();
        });
      } else {
        translateX.value = withTiming(0, { duration: durations.base, easing: easings.standard });
      }
    });

  const toneColor = (() => {
    switch (toast.tone) {
      case "success":
        return tokens.success;
      case "danger":
        return tokens.danger;
      case "warn":
        return tokens.warn;
      default:
        return tokens.borderStrong;
    }
  })();

  const style = useAnimatedStyle(() => ({
    opacity: opacity.value,
    transform: [{ translateY: translateY.value }, { translateX: translateX.value }],
  }));

  return (
    <GestureDetector gesture={swipe}>
      <Animated.View
        style={[styles.toast, { backgroundColor: tokens.bg3, borderLeftColor: toneColor }, style]}
        accessibilityRole="alert"
        accessibilityLiveRegion="polite"
        accessibilityLabel={toast.message}
        accessibilityActions={[{ name: "dismiss", label: "Dismiss notification" }]}
        onAccessibilityAction={(event) => {
          if (event.nativeEvent.actionName === "dismiss") dismiss();
        }}
      >
        <Text style={[type.body, styles.message, { color: tokens.ink }]}>{toast.message}</Text>
        <Pressable onPress={dismiss} accessibilityRole="button" accessibilityLabel="Dismiss notification" hitSlop={12}>
          <Text style={[type.bodyBold, { color: tokens.ink2 }]}>×</Text>
        </Pressable>
      </Animated.View>
    </GestureDetector>
  );
}

const styles = StyleSheet.create({
  toast: {
    borderRadius: radii.radius12,
    borderLeftWidth: 3,
    paddingHorizontal: space.space16,
    paddingVertical: space.space12,
    marginHorizontal: space.space16,
    marginBottom: space.space8,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space12,
  },
  message: { flex: 1 },
});
