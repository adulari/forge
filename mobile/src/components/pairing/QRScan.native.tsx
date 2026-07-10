// Native QR scanner for the Connect screen (DESIGN_SYSTEM.md §6 `QRScanFrame`,
// ARCHITECTURE.md §2 platform escape hatch "QR pairing"). 240pt rounded-16
// reticle with ember corner strokes; a scan flash + selection haptic fires the
// instant a QR code decodes — the parent (`connect.tsx`) owns testing the
// scanned URL against the daemon and reports back via `paused` while it does.
import { CameraView, useCameraPermissions, type BarcodeScanningResult } from "expo-camera";
import React, { useCallback, useEffect, useRef } from "react";
import { Linking, Pressable, StyleSheet, Text, View } from "react-native";
import Animated, {
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withSequence,
  withTiming,
} from "react-native-reanimated";

import { haptics } from "../../lib/haptics";
import { durations, easings } from "../../theme/motion";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space, tapTarget } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";

export interface QRScanProps {
  onScanned: (data: string) => void;
  /** Stop reacting to new frames — set while the parent is testing a scanned URL. */
  paused?: boolean;
}

const FRAME = 240;
const CORNER = 28;
const CORNER_W = 3;

export function QRScan({ onScanned, paused = false }: QRScanProps) {
  const tokens = useTokens();
  const reduced = useReducedMotion();
  const [permission, requestPermission] = useCameraPermissions();
  const lockRef = useRef(false);
  const flash = useSharedValue(0);

  useEffect(() => {
    if (permission && !permission.granted && permission.canAskAgain) {
      requestPermission();
    }
  }, [permission, requestPermission]);

  useEffect(() => {
    // Re-arm once the parent resumes us (e.g. after a failed test on the last scan).
    if (!paused) lockRef.current = false;
  }, [paused]);

  const onBarcode = useCallback(
    (result: BarcodeScanningResult) => {
      if (lockRef.current || paused) return;
      lockRef.current = true;
      haptics.select();
      flash.value = reduced
        ? 0
        : withSequence(
            withTiming(0.55, { duration: durations.instant, easing: easings.standard }),
            withTiming(0, { duration: durations.base, easing: easings.standard }),
          );
      onScanned(result.data);
    },
    [onScanned, paused, reduced, flash],
  );

  const flashStyle = useAnimatedStyle(() => ({ opacity: flash.value }));

  if (!permission || !permission.granted) {
    const canRetry = !permission || permission.canAskAgain;
    return (
      <View
        style={[styles.frame, styles.center, { borderColor: tokens.border, backgroundColor: tokens.bg3 }]}
        accessibilityRole="text"
      >
        <Text style={[typeScale.sub, styles.centerText, { color: tokens.ink2 }]}>
          {permission == null
            ? "checking camera access…"
            : "camera access is off — paste the connect url below instead."}
        </Text>
        {permission && !canRetry ? (
          <Pressable
            onPress={() => Linking.openSettings()}
            accessibilityRole="button"
            accessibilityLabel="Open Settings"
            style={styles.action}
          >
            <Text style={[typeScale.sub, { color: tokens.accent }]}>open settings</Text>
          </Pressable>
        ) : null}
      </View>
    );
  }

  return (
    <View
      style={[styles.frame, { borderRadius: radii.radius16, borderColor: tokens.border }]}
      accessibilityRole="image"
      accessibilityLabel="QR scanner. Point the camera at the connect QR code printed by forge serve."
    >
      <CameraView
        style={StyleSheet.absoluteFill}
        facing="back"
        barcodeScannerSettings={{ barcodeTypes: ["qr"] }}
        onBarcodeScanned={paused ? undefined : onBarcode}
      />
      <View pointerEvents="none" style={styles.reticleWrap}>
        <View style={styles.reticle}>
          <View style={[styles.corner, styles.cornerTL, { borderColor: tokens.accent }]} />
          <View style={[styles.corner, styles.cornerTR, { borderColor: tokens.accent }]} />
          <View style={[styles.corner, styles.cornerBL, { borderColor: tokens.accent }]} />
          <View style={[styles.corner, styles.cornerBR, { borderColor: tokens.accent }]} />
        </View>
      </View>
      <Animated.View
        pointerEvents="none"
        style={[StyleSheet.absoluteFill, { backgroundColor: tokens.accent }, flashStyle]}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  frame: {
    width: FRAME,
    height: FRAME,
    alignSelf: "center",
    overflow: "hidden",
    borderWidth: StyleSheet.hairlineWidth,
  },
  center: { alignItems: "center", justifyContent: "center", padding: space.space16, gap: space.space8 },
  centerText: { textAlign: "center" },
  action: { minHeight: tapTarget, justifyContent: "center" },
  reticleWrap: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
    alignItems: "center",
    justifyContent: "center",
  },
  reticle: { width: FRAME - 16, height: FRAME - 16 },
  corner: { position: "absolute", width: CORNER, height: CORNER },
  cornerTL: { top: 0, left: 0, borderLeftWidth: CORNER_W, borderTopWidth: CORNER_W, borderTopLeftRadius: radii.radius12 },
  cornerTR: { top: 0, right: 0, borderRightWidth: CORNER_W, borderTopWidth: CORNER_W, borderTopRightRadius: radii.radius12 },
  cornerBL: {
    bottom: 0,
    left: 0,
    borderLeftWidth: CORNER_W,
    borderBottomWidth: CORNER_W,
    borderBottomLeftRadius: radii.radius12,
  },
  cornerBR: {
    bottom: 0,
    right: 0,
    borderRightWidth: CORNER_W,
    borderBottomWidth: CORNER_W,
    borderBottomRightRadius: radii.radius12,
  },
});
