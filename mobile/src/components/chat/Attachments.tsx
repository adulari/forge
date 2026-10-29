// Attachment rendering shared by MessageRow: image thumbnails (tap opens the shared image
// viewer: zoom, copy, save) and file chips (name + icon) for non-images. Used both for the
// optimistic "just sent" bubble (client-local `SentAttachment[]`, local picker URI) and for
// `@path` mentions recovered from a persisted history row's content (see MessageRow.tsx): there
// the `uri` is a `GET /api/upload` URL instead of a local file URI.
import { FileText } from "lucide-react-native";
import React, { useMemo } from "react";
import { Image, Pressable, StyleSheet, Text, View } from "react-native";

import type { ViewerImage } from "../../lib/imageTypes";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { useImageViewer } from "../ds/ImageViewer";
import { isPdfName, type SentAttachment } from "./attach";
import { PdfPage } from "./AttachmentTray";

const THUMB = 72;

export function AttachmentRow({ attachments }: { attachments: SentAttachment[] }) {
  const tokens = useTokens();
  const gallery = useMemo(
    () => attachments.filter((a) => a.image && a.uri).map((a): ViewerImage => ({ uri: a.uri as string, name: a.name })),
    [attachments],
  );
  const viewer = useImageViewer(gallery);

  if (attachments.length === 0) return null;

  return (
    <>
      <View style={styles.row}>
        {attachments.map((a) =>
          a.image && a.uri ? (
            <Pressable
              key={a.id}
              onPress={() => viewer.open(gallery.findIndex((g) => g.uri === a.uri))}
              accessibilityRole="button"
              accessibilityLabel={`view image ${a.name}`}
            >
              <Image
                source={{ uri: a.uri }}
                style={[styles.thumb, { borderColor: tokens.border }]}
                resizeMode="cover"
              />
            </Pressable>
          ) : (
            <View
              key={a.id}
              style={[styles.fileChip, { backgroundColor: tokens.bg3, borderColor: tokens.border }]}
            >
              {isPdfName(a.name) ? <PdfPage size={28} /> : <FileText size={14} strokeWidth={1.75} color={tokens.ink2} />}
              <Text style={[type.meta, styles.fileName, { color: tokens.ink2 }]} numberOfLines={1}>
                {a.name}
              </Text>
            </View>
          ),
        )}
      </View>
      {viewer.element}
    </>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", flexWrap: "wrap", gap: space.space8, marginBottom: space.space8 },
  thumb: {
    width: THUMB,
    height: THUMB,
    borderRadius: radii.radius8,
    borderWidth: StyleSheet.hairlineWidth,
  },
  fileChip: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space4,
    paddingHorizontal: space.space8,
    paddingVertical: space.space4,
    borderRadius: radii.radius8,
    borderWidth: StyleSheet.hairlineWidth,
    maxWidth: 220,
  },
  fileName: { flexShrink: 1 },
});
