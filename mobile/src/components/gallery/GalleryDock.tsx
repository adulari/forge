// The thread's Gallery dock: every screenshot the session's browser and device tools captured, with
// the tool, the caption and when. Thumbnails are downsampled by the daemon; a tile opens the shared
// image viewer, which steps through the set.
import { Images } from "lucide-react-native";
import React, { useMemo, useState } from "react";
import { ActivityIndicator, Image, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { useAuth } from "../../lib/auth";
import { galleryEntries } from "../../lib/galleryModel";
import type { ViewerImage } from "../../lib/imageTypes";
import { evidenceImageUrl } from "../../lib/mediaApi";
import { useEvidenceImages } from "../../lib/mediaQueries";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { EmptyState } from "../ds/EmptyState";
import { useImageViewer } from "../ds/ImageViewer";
import { RelativeTime } from "../ds/RelativeTime";
import { MIN_TILE, GAP, tileLayout } from "./tileLayoutModel";

const THUMB_WIDTH = 360;
export function GalleryDock({ sessionId }: { sessionId: string }) {
  const tokens = useTokens();
  const { baseUrl } = useAuth();
  const evidence = useEvidenceImages(sessionId, true);
  const entries = useMemo(() => galleryEntries(evidence.data?.items ?? []), [evidence.data]);
  const images = useMemo<ViewerImage[]>(
    () =>
      baseUrl
        ? entries.map((entry) => ({
            uri: evidenceImageUrl(baseUrl, sessionId, entry.file),
            name: entry.title,
            caption: entry.page ? `${entry.tool} · ${entry.page}` : entry.tool,
          }))
        : [],
    [baseUrl, entries, sessionId],
  );
  const viewer = useImageViewer(images);
  const [inner, setInner] = useState(0);
  const { tile } = tileLayout(inner || 2 * MIN_TILE + GAP);

  if (evidence.isLoading) {
    return (
      <View style={styles.center}>
        <ActivityIndicator color={tokens.accent} />
      </View>
    );
  }
  if (!baseUrl || entries.length === 0) {
    return (
      <EmptyState
        icon={Images}
        message="No images yet. Screenshots from the browser and device tools show up here as they are taken."
      />
    );
  }

  return (
    <>
      <ScrollView contentContainerStyle={styles.grid}>
        <Text style={[typeScale.meta, styles.count, { color: tokens.ink3 }]}>
          {entries.length} {entries.length === 1 ? "image" : "images"}
        </Text>
        <View style={styles.tiles} onLayout={(event) => setInner(event.nativeEvent.layout.width)}>
          {entries.map((entry, index) => (
            <Pressable
              key={entry.file}
              onPress={() => viewer.open(index)}
              accessibilityRole="button"
              accessibilityLabel={`${entry.title}, ${entry.tool}, open image`}
              style={({ pressed, hovered }: { pressed: boolean; hovered?: boolean }) => [
                styles.tile,
                {
                  width: tile,
                  borderColor: hovered || pressed ? tokens.borderStrong : tokens.border,
                  backgroundColor: tokens.bg2,
                },
              ]}
            >
              <View style={[styles.thumbBox, { backgroundColor: tokens.bg0 }]}>
                <Image
                  source={{ uri: evidenceImageUrl(baseUrl, sessionId, entry.file, THUMB_WIDTH) }}
                  resizeMode="cover"
                  style={styles.thumb}
                />
              </View>
              <View style={styles.caption}>
                <Text style={[typeScale.sub, { color: tokens.ink }]} numberOfLines={1}>
                  {entry.title}
                </Text>
                <View style={styles.meta}>
                  <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]} numberOfLines={1}>
                    {entry.tool}
                  </Text>
                  {Number.isFinite(entry.at) ? <RelativeTime timestampMs={entry.at} /> : null}
                </View>
              </View>
            </Pressable>
          ))}
        </View>
      </ScrollView>
      {viewer.element}
    </>
  );
}

const styles = StyleSheet.create({
  center: { flex: 1, alignItems: "center", justifyContent: "center" },
  grid: { padding: space.space12, gap: space.space8 },
  count: { paddingBottom: space.space4 },
  tiles: { flexDirection: "row", flexWrap: "wrap", gap: GAP },
  tile: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius8,
    overflow: "hidden",
  },
  thumbBox: { aspectRatio: 4 / 3, width: "100%" },
  thumb: { width: "100%", height: "100%" },
  caption: { padding: space.space8, gap: 2 },
  meta: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: space.space8 },
});
