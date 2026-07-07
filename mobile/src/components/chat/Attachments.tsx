// Attachment rendering shared by MessageRow: image thumbnails (tap → full-size, since there's
// no existing lightbox in the app to defer to) and file chips (name + icon) for non-images.
// Used both for the optimistic "just sent" bubble (client-local `SentAttachment[]`) and for
// text-file mentions recovered from a persisted history row's content (see MessageRow.tsx —
// the daemon never persists image attachments, only a leading `@path` mention for text files).
import { FileText } from "lucide-react-native";
import React, { useState } from "react";
import { Image, Modal, Pressable, StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import type { SentAttachment } from "./attach";

const THUMB = 72;

export function AttachmentRow({ attachments }: { attachments: SentAttachment[] }) {
  const tokens = useTokens();
  const [preview, setPreview] = useState<SentAttachment | null>(null);

  if (attachments.length === 0) return null;

  return (
    <>
      <View style={styles.row}>
        {attachments.map((a) =>
          a.image && a.uri ? (
            <Pressable
              key={a.id}
              onPress={() => setPreview(a)}
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
              <FileText size={14} strokeWidth={1.75} color={tokens.ink2} />
              <Text style={[type.meta, styles.fileName, { color: tokens.ink2 }]} numberOfLines={1}>
                {a.name}
              </Text>
            </View>
          ),
        )}
      </View>

      <Modal
        visible={preview !== null}
        transparent
        animationType="fade"
        onRequestClose={() => setPreview(null)}
      >
        <Pressable
          style={[styles.backdrop, { backgroundColor: tokens.overlayScrim }]}
          onPress={() => setPreview(null)}
          accessibilityRole="button"
          accessibilityLabel="close image preview"
        >
          {preview?.uri ? (
            <Image source={{ uri: preview.uri }} style={styles.full} resizeMode="contain" />
          ) : null}
        </Pressable>
      </Modal>
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
    borderRadius: radii.radiusPill,
    borderWidth: StyleSheet.hairlineWidth,
    maxWidth: 220,
  },
  fileName: { flexShrink: 1 },
  backdrop: { flex: 1, alignItems: "center", justifyContent: "center" },
  full: { width: "100%", height: "100%" },
});
