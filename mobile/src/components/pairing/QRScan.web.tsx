// Web QR scanner for the Connect screen (DESIGN_SYSTEM.md §6 `QRScanFrame`,
// ARCHITECTURE.md §2 platform escape hatch "QR pairing"). Uses the browser
// Shape Detection API (`BarcodeDetector`) over a `getUserMedia` video stream —
// no native module, no bundle-size cost. When either isn't available (Firefox
// and Safari still lack `BarcodeDetector`; camera can be denied, missing, or
// blocked by an insecure context) this falls back to the same "scan on your
// phone, or paste the connect url below" hint the static placeholder used —
// paste-the-url always works regardless of browser support.
import { ScanLine } from "lucide-react-native";
import React, { useCallback, useEffect, useRef, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
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
  /** Starts the scanner only after deliberate user activation. */
  enabled?: boolean;
  /** Stop reacting to new frames — set while the parent is testing a scanned URL.
   * On web this also tears the MediaStream down so the browser's camera-active
   * indicator turns off while a scan is being tested. */
  paused?: boolean;
}

const FRAME = 240;
const CORNER = 28;
const CORNER_W = 3;
const DETECT_INTERVAL_MS = 200;

// The Shape Detection API isn't in TS's `lib.dom` yet (2026) — declared locally
// rather than as an ambient global so it can't collide if/when it lands there.
interface DetectedBarcode {
  rawValue: string;
}
interface BarcodeDetectorLike {
  detect(source: CanvasImageSource): Promise<DetectedBarcode[]>;
}
interface BarcodeDetectorCtor {
  new (options: { formats: string[] }): BarcodeDetectorLike;
}

function getBarcodeDetectorCtor(): BarcodeDetectorCtor | null {
  if (typeof window === "undefined") return null;
  return (window as unknown as { BarcodeDetector?: BarcodeDetectorCtor }).BarcodeDetector ?? null;
}

type UnavailableReason = "unsupported" | "insecure" | "no-camera" | "error";

type ScanState =
  | { kind: "requesting" }
  | { kind: "scanning" }
  | { kind: "denied" }
  | { kind: "unavailable"; reason: UnavailableReason };

const UNAVAILABLE_COPY: Record<UnavailableReason, string> = {
  unsupported:
    "camera scanning isn't supported in this browser — open forge on your phone to scan, or paste the connect url below.",
  insecure:
    "camera needs a secure (https) connection — open forge on your phone to scan, or paste the connect url below.",
  "no-camera": "no camera found — open forge on your phone to scan, or paste the connect url below.",
  error: "camera unavailable — open forge on your phone to scan, or paste the connect url below.",
};

const CAN_RETRY: Record<UnavailableReason, boolean> = {
  unsupported: false,
  insecure: false,
  "no-camera": true,
  error: true,
};

export function QRScan({ onScanned, enabled = false, paused = false }: QRScanProps) {
  const tokens = useTokens();
  const reduced = useReducedMotion();
  const [state, setState] = useState<ScanState>({ kind: "requesting" });
  const [retryTick, setRetryTick] = useState(0);
  const videoRef = useRef<HTMLVideoElement | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const detectorRef = useRef<BarcodeDetectorLike | null>(null);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lockRef = useRef(false);
  const onScannedRef = useRef(onScanned);
  const flash = useSharedValue(0);

  useEffect(() => {
    onScannedRef.current = onScanned;
  }, [onScanned]);

  useEffect(() => {
    const video = videoRef.current;
    const stream = streamRef.current;
    if (state.kind !== "scanning" || !video || !stream) return;
    video.srcObject = stream;
    void video.play().catch(() => {});
  }, [state.kind]);

  useEffect(() => {
    // Re-arm once the parent resumes us (e.g. after a failed test on the last scan).
    if (!paused) lockRef.current = false;
  }, [paused]);

  const stopStream = useCallback(() => {
    if (timeoutRef.current != null) {
      clearTimeout(timeoutRef.current);
      timeoutRef.current = null;
    }
    if (streamRef.current) {
      for (const track of streamRef.current.getTracks()) track.stop();
      streamRef.current = null;
    }
    if (videoRef.current) videoRef.current.srcObject = null;
    detectorRef.current = null;
  }, []);

  useEffect(() => {
    if (!enabled) {
      stopStream();
      return;
    }
    if (paused) {
      stopStream();
      return;
    }

    let cancelled = false;

    function scheduleDetect() {
      timeoutRef.current = setTimeout(async () => {
        if (cancelled) return;
        const video = videoRef.current;
        const detector = detectorRef.current;
        if (!video || !detector || lockRef.current || video.readyState < video.HAVE_ENOUGH_DATA) {
          scheduleDetect();
          return;
        }
        try {
          const results = await detector.detect(video);
          const value = results[0]?.rawValue;
          if (value && !lockRef.current && !cancelled) {
            lockRef.current = true;
            haptics.select();
            flash.value = reduced
              ? 0
              : withSequence(
                  withTiming(0.55, { duration: durations.instant, easing: easings.standard }),
                  withTiming(0, { duration: durations.base, easing: easings.standard }),
                );
            onScannedRef.current(value);
            return;
          }
        } catch {
          // transient mid-frame decode failures are expected — keep polling.
        }
        scheduleDetect();
      }, DETECT_INTERVAL_MS);
    }

    async function start() {
      if (typeof navigator === "undefined" || !navigator.mediaDevices?.getUserMedia) {
        setState({ kind: "unavailable", reason: "unsupported" });
        return;
      }
      if (typeof window !== "undefined" && window.isSecureContext === false) {
        setState({ kind: "unavailable", reason: "insecure" });
        return;
      }
      const DetectorCtor = getBarcodeDetectorCtor();
      if (!DetectorCtor) {
        setState({ kind: "unavailable", reason: "unsupported" });
        return;
      }

      setState({ kind: "requesting" });
      try {
        const stream = await navigator.mediaDevices.getUserMedia({
          video: { facingMode: "environment" },
          audio: false,
        });
        if (cancelled) {
          for (const track of stream.getTracks()) track.stop();
          return;
        }
        streamRef.current = stream;
        detectorRef.current = new DetectorCtor({ formats: ["qr_code"] });
        const video = videoRef.current;
        if (video) {
          video.srcObject = stream;
          await video.play().catch(() => {});
        }
        if (cancelled) return;
        setState({ kind: "scanning" });
        scheduleDetect();
      } catch (err) {
        if (cancelled) return;
        const name = (err as { name?: string } | undefined)?.name;
        if (name === "NotAllowedError" || name === "SecurityError") {
          setState({ kind: "denied" });
        } else if (name === "NotFoundError" || name === "OverconstrainedError") {
          setState({ kind: "unavailable", reason: "no-camera" });
        } else {
          setState({ kind: "unavailable", reason: "error" });
        }
      }
    }

    start();

    return () => {
      cancelled = true;
      stopStream();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled, paused, retryTick, stopStream]);

  const flashStyle = useAnimatedStyle(() => ({ opacity: flash.value }));
  const retry = () => setRetryTick((n) => n + 1);

  if (!enabled) {
    return <View style={[styles.hintFrame, { borderColor: tokens.border, backgroundColor: tokens.bg3, borderRadius: radii.radius16 }]} accessibilityRole="image" accessibilityLabel="QR scanner is off until you start scanning"><ScanLine size={24} strokeWidth={1.75} color={tokens.ink3} /><Text style={[typeScale.sub, styles.hint, { color: tokens.ink2 }]}>Camera stays off until you choose to scan.</Text></View>;
  }

  if (state.kind !== "scanning") {
    const text = state.kind === "requesting" ? "requesting camera access…" : state.kind === "denied" ? "camera blocked — allow access in your browser, or paste the connect url below." : UNAVAILABLE_COPY[state.reason];
    const canRetry = state.kind === "denied" || (state.kind === "unavailable" && CAN_RETRY[state.reason]);
    return (
      <View
        style={[styles.hintFrame, { borderColor: tokens.border, backgroundColor: tokens.bg3, borderRadius: radii.radius16 }]}
        accessibilityRole="text"
        accessibilityLabel={text}
      >
        <ScanLine size={24} strokeWidth={1.75} color={tokens.ink3} />
        <Text style={[typeScale.sub, styles.hint, { color: tokens.ink2 }]}>{text}</Text>
        {canRetry ? (
          <Pressable onPress={retry} accessibilityRole="button" accessibilityLabel="Try camera again" style={styles.action}>
            <Text style={[typeScale.sub, { color: tokens.accent }]}>try again</Text>
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
      <video
        ref={videoRef}
        muted
        playsInline
        autoPlay
        style={{ position: "absolute", top: 0, left: 0, right: 0, bottom: 0, width: "100%", height: "100%", objectFit: "cover" }}
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
    width: "100%",
    maxWidth: FRAME,
    aspectRatio: 1,
    alignSelf: "center",
    overflow: "hidden",
    borderWidth: StyleSheet.hairlineWidth,
  },
  hintFrame: {
    minHeight: 160,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: StyleSheet.hairlineWidth,
    padding: space.space24,
    gap: space.space12,
  },
  hint: { textAlign: "center", maxWidth: 300 },
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
