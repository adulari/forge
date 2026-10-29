// A full-window image viewer: fit-to-window by default, zoom with buttons, keys (+ - 0 1) or
// ctrl/cmd + wheel, pan by scrolling when zoomed, Copy and Save in the bar, and arrow keys to step
// through a set (a thread's gallery). Esc closes. Used by attachments, tool screenshots, images in
// replies and the gallery dock, so all of them behave the same.
import { ChevronLeft, ChevronRight, Copy, Download, Maximize2, X, ZoomIn, ZoomOut } from "lucide-react-native";
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ActivityIndicator, Image, Modal, Platform, Pressable, ScrollView, StyleSheet, Text, View, useWindowDimensions } from "react-native";

import { copyImage, saveImage } from "../../lib/imageActions";
import type { ViewerImage } from "../../lib/imageTypes";
import { fitScale, nextZoom } from "../../lib/imageZoom";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { useOverlayEscape } from "../../lib/useOverlayEscape";
import { type as typeScale } from "../../theme/typography";
import { IconButton } from "./IconButton";

export interface ImageViewerProps {
  visible: boolean;
  images: readonly ViewerImage[];
  index?: number;
  onIndexChange?: (index: number) => void;
  onClose: () => void;
}

type Notice = { text: string; failed: boolean } | null;

const NOTICE_MS = 2200;

export function ImageViewer({ visible, images, index = 0, onIndexChange, onClose }: ImageViewerProps) {
  const tokens = useTokens();
  const narrow = useWindowDimensions().width < 560;
  const image = images[Math.min(Math.max(index, 0), images.length - 1)] ?? null;
  const [natural, setNatural] = useState<{ width: number; height: number } | null>(null);
  const [stage, setStage] = useState({ width: 0, height: 0 });
  const [zoom, setZoom] = useState(1);
  const [failed, setFailed] = useState(false);
  const [notice, setNotice] = useState<Notice>(null);
  const [busy, setBusy] = useState(false);
  const noticeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const uri = image?.uri ?? null;
  useEffect(() => {
    setNatural(null);
    setFailed(false);
    setZoom(1);
    if (!uri) return;
    let live = true;
    Image.getSize(
      uri,
      (width, height) => live && setNatural({ width, height }),
      () => live && setFailed(true),
    );
    return () => {
      live = false;
    };
  }, [uri]);
  useEffect(() => () => { if (noticeTimer.current) clearTimeout(noticeTimer.current); }, []);

  const fit = natural ? fitScale(natural, stage) : 1;
  const shown = natural ? { width: natural.width * fit * zoom, height: natural.height * fit * zoom } : null;

  const say = useCallback((text: string, isFailure = false) => {
    setNotice({ text, failed: isFailure });
    if (noticeTimer.current) clearTimeout(noticeTimer.current);
    noticeTimer.current = setTimeout(() => setNotice(null), NOTICE_MS);
  }, []);

  const run = useCallback(
    async (action: (image: ViewerImage) => Promise<string | void>, done: string) => {
      if (!image || busy) return;
      setBusy(true);
      try {
        say((await action(image)) || done);
      } catch (error) {
        say(error instanceof Error ? error.message : "That did not work.", true);
      } finally {
        setBusy(false);
      }
    },
    [busy, image, say],
  );

  const step = useCallback(
    (direction: 1 | -1) => setZoom((current) => nextZoom(current, direction)),
    [],
  );
  const go = useCallback(
    (delta: number) => {
      const target = index + delta;
      if (target >= 0 && target < images.length) onIndexChange?.(target);
    },
    [images.length, index, onIndexChange],
  );

  useOverlayEscape(visible, onClose);
  useEffect(() => {
    if (!visible || Platform.OS !== "web") return;
    const onKey = (event: KeyboardEvent) => {
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      switch (event.key) {
        case "+": case "=": step(1); break;
        case "-": case "_": step(-1); break;
        case "0": setZoom(1); break;
        case "1": if (fit > 0) setZoom(1 / fit); break;
        case "ArrowLeft": go(-1); break;
        case "ArrowRight": go(1); break;
        default: return;
      }
      event.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [visible, step, go, fit]);

  const webWheel =
    Platform.OS === "web"
      ? ({
          onWheel: (event: { ctrlKey: boolean; metaKey: boolean; deltaY: number; preventDefault: () => void }) => {
            if (!event.ctrlKey && !event.metaKey) return;
            event.preventDefault();
            step(event.deltaY < 0 ? 1 : -1);
          },
          onDoubleClick: () => setZoom((current) => (current > 1 ? 1 : 2)),
        } as Record<string, unknown>)
      : null;

  const percent = Math.round(fit * zoom * 100);
  const many = images.length > 1;

  return (
    <Modal visible={visible && image != null} transparent animationType="fade" onRequestClose={onClose}>
      <View style={styles.backdrop} accessibilityViewIsModal>
        <View style={[StyleSheet.absoluteFill, styles.shade, { backgroundColor: tokens.bg0 }]} pointerEvents="none" />
        <View style={[styles.bar, narrow && styles.barNarrow, { backgroundColor: tokens.bg1, borderBottomColor: tokens.border }]}>
          <View style={[styles.title, narrow && styles.titleNarrow]}>
            <Text style={[typeScale.bodyBold, { color: tokens.ink }]} numberOfLines={1}>
              {image?.name}
            </Text>
            {image?.caption ? (
              <Text style={[typeScale.meta, { color: tokens.ink3 }]} numberOfLines={1}>
                {image.caption}
              </Text>
            ) : null}
          </View>
          {notice ? (
            <Text
              accessibilityLiveRegion="polite"
              style={[typeScale.meta, { color: notice.failed ? tokens.danger : tokens.success }]}
              numberOfLines={1}
            >
              {notice.text}
            </Text>
          ) : null}
          {many ? (
            <>
              <IconButton
                icon={<ChevronLeft size={18} color={index > 0 ? tokens.ink2 : tokens.ink4} />}
                onPress={() => go(-1)}
                disabled={index <= 0}
                accessibilityLabel="Previous image"
              />
              <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>
                {index + 1}/{images.length}
              </Text>
              <IconButton
                icon={<ChevronRight size={18} color={index < images.length - 1 ? tokens.ink2 : tokens.ink4} />}
                onPress={() => go(1)}
                disabled={index >= images.length - 1}
                accessibilityLabel="Next image"
              />
            </>
          ) : null}
          <IconButton icon={<ZoomOut size={18} color={tokens.ink2} />} onPress={() => step(-1)} accessibilityLabel="Zoom out" />
          <Pressable
            onPress={() => setZoom(1)}
            accessibilityRole="button"
            accessibilityLabel={`Zoom ${percent} percent, fit to window`}
            style={styles.percent}
          >
            <Text style={[typeScale.monoMeta, { color: tokens.ink2 }]}>{percent}%</Text>
          </Pressable>
          <IconButton icon={<ZoomIn size={18} color={tokens.ink2} />} onPress={() => step(1)} accessibilityLabel="Zoom in" />
          <IconButton icon={<Maximize2 size={16} color={tokens.ink2} />} onPress={() => setZoom(1)} accessibilityLabel="Fit to window" />
          <IconButton
            icon={<Copy size={17} color={tokens.ink2} />}
            onPress={() => void run(async (target) => { await copyImage(target); }, "Copied image")}
            disabled={busy}
            accessibilityLabel="Copy image"
          />
          <IconButton
            icon={<Download size={17} color={tokens.ink2} />}
            onPress={() => void run(saveImage, "Saved")}
            disabled={busy}
            accessibilityLabel="Save image"
          />
          <IconButton icon={<X size={18} color={tokens.ink2} />} onPress={onClose} accessibilityLabel="Close image viewer" />
        </View>

        <View
          style={styles.stage}
          onLayout={(event) => setStage({ width: event.nativeEvent.layout.width, height: event.nativeEvent.layout.height })}
          {...webWheel}
        >
          <Pressable
            style={StyleSheet.absoluteFill}
            onPress={onClose}
            accessible={false}
            tabIndex={-1}
          />
          {failed ? (
            <View style={styles.center}>
              <Text style={[typeScale.body, { color: tokens.ink2 }]}>This image could not be loaded.</Text>
            </View>
          ) : !shown || !image ? (
            <View style={styles.center}>
              <ActivityIndicator color={tokens.accent} />
            </View>
          ) : (
            <ScrollView
              style={styles.fill}
              contentContainerStyle={[styles.scrollContent, { minHeight: stage.height }]}
              maximumZoomScale={4}
              minimumZoomScale={1}
              showsVerticalScrollIndicator={shown.height > stage.height}
              pinchGestureEnabled
            >
              <ScrollView
                horizontal
                contentContainerStyle={[styles.scrollContent, { minWidth: stage.width }]}
                showsHorizontalScrollIndicator={shown.width > stage.width}
              >
                <Image
                  source={{ uri: image.uri }}
                  style={{ width: shown.width, height: shown.height }}
                  resizeMode="contain"
                  accessibilityLabel={image.caption ? `${image.name}: ${image.caption}` : image.name}
                />
              </ScrollView>
            </ScrollView>
          )}
        </View>
      </View>
    </Modal>
  );
}

export function useImageViewer(images: readonly ViewerImage[]) {
  const [index, setIndex] = useState<number | null>(null);
  const element = useMemo(
    () => (
      <ImageViewer
        visible={index != null}
        images={images}
        index={index ?? 0}
        onIndexChange={setIndex}
        onClose={() => setIndex(null)}
      />
    ),
    [images, index],
  );
  return { open: setIndex, element };
}

const styles = StyleSheet.create({
  backdrop: { flex: 1 },
  shade: { opacity: 0.95 },
  bar: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space4,
    paddingHorizontal: space.space12,
    paddingVertical: space.space4,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  barNarrow: { flexWrap: "wrap", justifyContent: "flex-end" },
  title: { flex: 1, minWidth: 0, paddingRight: space.space8 },
  titleNarrow: { flexBasis: "100%", paddingTop: space.space4 },
  percent: {
    minWidth: 48,
    height: 32,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radii.radius4,
  },
  stage: { flex: 1, minHeight: 0 },
  fill: { flex: 1 },
  center: { flex: 1, alignItems: "center", justifyContent: "center" },
  scrollContent: { flexGrow: 1, alignItems: "center", justifyContent: "center", padding: space.space12 },
});
