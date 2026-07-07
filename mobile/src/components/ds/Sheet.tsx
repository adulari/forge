// DESIGN_SYSTEM.md §6 Containers / §5.2 Anvil — gesture-driven bottom sheet.
// Native: follows the finger 1:1 via react-native-gesture-handler, `sheet`
// spring to snap points, scrim opacity tracks progress. Web: transform
// transition 260ms `standard`, Esc/scrim closes. Platform branch lives inside
// this one file per BUILD_ORDER T1.3.
import React, { useCallback, useEffect, useMemo, useState } from "react";
import { BackHandler, Modal, Platform, Pressable, StyleSheet, View, useWindowDimensions } from "react-native";
import { Gesture, GestureDetector } from "react-native-gesture-handler";
import Animated, {
  cancelAnimation,
  runOnJS,
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withSpring,
  withTiming,
} from "react-native-reanimated";

import { durations, easings, springs } from "../../theme/motion";
import { useTheme, useTokens } from "../../theme/ThemeProvider";
import { depthDark, depthLight, radii, space } from "../../theme/tokens";

export interface SheetProps {
  visible: boolean;
  onClose: () => void;
  children: React.ReactNode;
  /**
   * Snap points as fractions (0-1] of the sheet's max height, rest state first.
   * Default `[1]` (single full-height rest position).
   */
  snapPoints?: number[];
  /** Max height as a fraction of window height. Default 0.9. */
  maxHeightRatio?: number;
  accessibilityLabel?: string;
}

const GRABBER_W = 36;
const GRABBER_H = 4;
const CLOSE_VELOCITY = 800;
const CLOSE_DISTANCE_RATIO = 0.3;

export function Sheet({ visible, onClose, children, snapPoints = [1], maxHeightRatio = 0.9, accessibilityLabel }: SheetProps) {
  const tokens = useTokens();
  const { scheme } = useTheme();
  const { height: windowHeight } = useWindowDimensions();
  const reduced = useReducedMotion();
  const depth = scheme === "dark" ? depthDark : depthLight;

  const sheetHeight = windowHeight * maxHeightRatio;
  const snapY = useMemo(
    () => [...snapPoints].map((p) => sheetHeight * (1 - Math.max(0, Math.min(1, p)))).sort((a, b) => a - b),
    [snapPoints, sheetHeight],
  );
  const restY = snapY[0] ?? 0;
  const closedY = sheetHeight;

  const translateY = useSharedValue(closedY);
  const scrimOpacity = useSharedValue(0);
  const startY = useSharedValue(0);

  const [mounted, setMounted] = useState(visible);

  const close = useCallback(() => onClose(), [onClose]);

  useEffect(() => {
    if (visible) setMounted(true);
  }, [visible]);

  useEffect(() => {
    if (!mounted) return;
    if (visible) {
      if (reduced) {
        translateY.value = restY;
        scrimOpacity.value = 1;
        return;
      }
      translateY.value = withSpring(restY, springs.sheet);
      scrimOpacity.value = withTiming(1, { duration: durations.fast, easing: easings.standard });
    } else {
      if (reduced) {
        translateY.value = closedY;
        scrimOpacity.value = 0;
        setMounted(false);
        return;
      }
      translateY.value = withTiming(closedY, { duration: durations.sheet, easing: easings.exit }, (finished) => {
        if (finished) runOnJS(setMounted)(false);
      });
      scrimOpacity.value = withTiming(0, { duration: durations.fast, easing: easings.exit });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible, mounted, reduced, restY, closedY]);

  // Android hardware back closes the sheet.
  useEffect(() => {
    if (!visible || Platform.OS !== "android") return;
    const sub = BackHandler.addEventListener("hardwareBackPress", () => {
      close();
      return true;
    });
    return () => sub.remove();
  }, [visible, close]);

  // Web: Esc closes.
  useEffect(() => {
    if (!visible || Platform.OS !== "web") return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [visible, close]);

  const farthestY = snapY[snapY.length - 1] ?? closedY;

  const pan = Gesture.Pan()
    .enabled(Platform.OS !== "web")
    .onStart(() => {
      startY.value = translateY.value;
    })
    .onUpdate((e) => {
      const next = startY.value + e.translationY;
      translateY.value = Math.max(farthestY, next);
      scrimOpacity.value = 1 - Math.min(1, translateY.value / sheetHeight);
    })
    .onEnd((e) => {
      const pastCloseDistance = translateY.value > sheetHeight * CLOSE_DISTANCE_RATIO;
      if (e.velocityY > CLOSE_VELOCITY || pastCloseDistance) {
        translateY.value = withTiming(closedY, { duration: durations.sheet, easing: easings.exit }, (finished) => {
          if (finished) runOnJS(setMounted)(false);
        });
        scrimOpacity.value = withTiming(0, { duration: durations.fast, easing: easings.exit });
        runOnJS(close)();
        return;
      }
      let nearest = snapY[0];
      let bestDist = Math.abs(translateY.value - snapY[0]);
      for (const y of snapY) {
        const d = Math.abs(translateY.value - y);
        if (d < bestDist) {
          bestDist = d;
          nearest = y;
        }
      }
      translateY.value = withSpring(nearest, springs.sheet);
      scrimOpacity.value = withTiming(1 - nearest / sheetHeight, { duration: durations.base, easing: easings.standard });
    });

  const sheetStyle = useAnimatedStyle(() => ({ transform: [{ translateY: translateY.value }] }));
  const scrimStyle = useAnimatedStyle(() => ({ opacity: scrimOpacity.value }));

  useEffect(() => {
    return () => {
      cancelAnimation(translateY);
      cancelAnimation(scrimOpacity);
    };
  }, [translateY, scrimOpacity]);

  if (!mounted) return null;

  const body = (
    <View style={StyleSheet.absoluteFill} pointerEvents={visible ? "auto" : "none"}>
      <Animated.View style={[StyleSheet.absoluteFill, { backgroundColor: tokens.overlayScrim }, scrimStyle]}>
        <Pressable
          style={StyleSheet.absoluteFill}
          onPress={close}
          accessibilityRole="button"
          accessibilityLabel="Close"
        />
      </Animated.View>
      <GestureDetector gesture={pan}>
        <Animated.View
          style={[
            styles.sheet,
            {
              backgroundColor: tokens.bg2,
              height: sheetHeight,
              borderTopLeftRadius: radii.radius16,
              borderTopRightRadius: radii.radius16,
            },
            depth.sheet,
            Platform.OS === "web" && styles.webTransition,
            sheetStyle,
          ]}
          accessibilityViewIsModal
          accessibilityLabel={accessibilityLabel}
        >
          <View style={styles.grabberRow}>
            <View style={[styles.grabber, { backgroundColor: tokens.border }]} />
          </View>
          {children}
        </Animated.View>
      </GestureDetector>
    </View>
  );

  return (
    <Modal visible={mounted} transparent animationType="none" onRequestClose={close} statusBarTranslucent>
      {body}
    </Modal>
  );
}

const styles = StyleSheet.create({
  sheet: { position: "absolute", left: 0, right: 0, bottom: 0 },
  webTransition: {
    // @ts-expect-error react-native-web-only CSS transition property
    transitionProperty: "transform",
    transitionDuration: "260ms",
    transitionTimingFunction: "cubic-bezier(0.2, 0, 0, 1)",
  },
  grabberRow: { alignItems: "center", paddingVertical: space.space12 },
  grabber: { width: GRABBER_W, height: GRABBER_H, borderRadius: 999 },
});
