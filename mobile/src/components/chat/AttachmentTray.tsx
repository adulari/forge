// The composer's pending attachments: image thumbnails, a page tile for PDFs, a file glyph for
// anything else. A finished upload that survived a reload has no local preview, so an image
// falls back to the host's copy (`previewUri`).
import { FileText, X } from "lucide-react-native";
import React from "react";
import { ActivityIndicator, Image, ScrollView, StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";
import { type Attachment, isPdfName } from "./attach";
import { MiniPressable } from "./MiniPressable";

const THUMB = 40;

export function PdfPage({ size = THUMB }: { size?: number }) {
  const tokens = useTokens();
  const small = size < 36;
  return (
    <View
      style={[
        styles.page,
        small ? styles.pageSmall : null,
        { width: size * 0.78, height: size, backgroundColor: tokens.bg0, borderColor: tokens.borderStrong },
      ]}
    >
      {small ? null : (
        <>
          <View style={[styles.pageRule, { backgroundColor: tokens.border }]} />
          <View style={[styles.pageRule, styles.pageRuleShort, { backgroundColor: tokens.border }]} />
        </>
      )}
      <Text
        numberOfLines={1}
        style={[styles.pageLabel, small ? styles.pageLabelSmall : null, { color: tokens.accent }]}
      >
        PDF
      </Text>
    </View>
  );
}

function Thumb({ attachment, previewUri }: { attachment: Attachment; previewUri?: string }) {
  const tokens = useTokens();
  const uri = attachment.uri ?? previewUri;
  if (attachment.image && uri) {
    return <Image source={{ uri }} style={[styles.thumb, { borderColor: tokens.border }]} resizeMode="cover" />;
  }
  if (isPdfName(attachment.name)) return <PdfPage />;
  return (
    <View style={[styles.thumb, styles.fileThumb, { backgroundColor: tokens.bg3, borderColor: tokens.border }]}>
      <FileText size={18} strokeWidth={1.75} color={tokens.ink3} />
    </View>
  );
}

export interface AttachmentTrayProps {
  attachments: Attachment[];
  onRemove: (id: string) => void;
  /** Where the host keeps an uploaded image, for a preview after a reload. */
  previewUriFor?: (attachment: Attachment) => string | undefined;
}

export function AttachmentTray({ attachments, onRemove, previewUriFor }: AttachmentTrayProps) {
  const tokens = useTokens();
  return (
    <ScrollView
      horizontal
      showsHorizontalScrollIndicator={false}
      style={styles.scroll}
      contentContainerStyle={styles.row}
      keyboardShouldPersistTaps="handled"
      accessibilityLabel="Pending attachments"
    >
      {attachments.map((a) => {
        const status = a.state === "uploading" ? "uploading…" : a.state === "error" ? "failed" : null;
        return (
          <View
            key={a.id}
            style={[
              styles.tile,
              { backgroundColor: tokens.bg2, borderColor: a.state === "error" ? tokens.danger : tokens.border },
            ]}
            testID={`attachment-${a.id}`}
          >
            <Thumb attachment={a} previewUri={previewUriFor?.(a)} />
            <View style={styles.meta}>
              <Text style={[type.meta, { color: tokens.ink }]} numberOfLines={1}>
                {a.name}
              </Text>
              <Text style={[type.monoMeta, { color: a.state === "error" ? tokens.danger : tokens.ink3 }]} numberOfLines={1}>
                {status ?? (isPdfName(a.name) ? "pdf" : a.image ? "image" : "file")}
              </Text>
            </View>
            {a.state === "uploading" ? <ActivityIndicator size="small" color={tokens.ink3} /> : null}
            <MiniPressable
              onPress={() => onRemove(a.id)}
              accessibilityRole="button"
              accessibilityLabel={`remove ${a.name}`}
              hitSlop={8}
              style={styles.remove}
            >
              <X size={14} strokeWidth={1.75} color={tokens.ink3} />
            </MiniPressable>
          </View>
        );
      })}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  scroll: { flexGrow: 0, flexShrink: 0 },
  row: { flexDirection: "row", gap: space.space8, paddingRight: space.space12 },
  tile: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    maxWidth: 240,
    minWidth: 150,
    padding: space.space4,
    paddingRight: space.space4,
    borderWidth: StyleSheet.hairlineWidth * 2,
    borderRadius: radii.radius8,
  },
  thumb: { width: THUMB, height: THUMB, borderRadius: radii.radius4, borderWidth: StyleSheet.hairlineWidth },
  fileThumb: { alignItems: "center", justifyContent: "center" },
  meta: { flex: 1, minWidth: 0 },
  remove: {
    width: 24,
    height: 24,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radii.radius4,
  },
  page: {
    borderWidth: 1,
    borderRadius: 3,
    padding: 5,
    gap: 3,
    justifyContent: "flex-start",
  },
  pageSmall: { padding: 0, alignItems: "center", justifyContent: "center" },
  pageRule: { height: 2, borderRadius: 1, width: "100%" },
  pageRuleShort: { width: "60%" },
  pageLabel: { fontFamily: monoFamily.bold, fontSize: 9, lineHeight: 11, marginTop: "auto" },
  pageLabelSmall: { fontSize: 6.5, lineHeight: 8, marginTop: 0 },
});
