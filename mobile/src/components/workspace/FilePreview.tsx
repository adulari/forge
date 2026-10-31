import React, { useMemo } from "react";
import { Image, Platform, ScrollView, StyleSheet, Text, View } from "react-native";

import { useAuth } from "../../lib/auth";
import { workspacePdfUrl } from "../../lib/mediaApi";
import { supportsDirectDaemonEndpoints } from "../../lib/transport";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Markdown } from "../chat/Markdown";

/** The browser renders SVG in an <img>; the native Image component does not. */
export const CAN_RENDER_SVG = Platform.OS === "web";

export function svgDataUri(svg: string): string {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

export function MarkdownPreview({ content }: { content: string }) {
  const tokens = useTokens();
  return (
    <ScrollView
      style={[styles.fill, { backgroundColor: tokens.bg0 }]}
      contentContainerStyle={styles.prose}
      accessibilityLabel="Markdown preview"
    >
      {content.trim() ? (
        <Markdown content={content} />
      ) : (
        <Text style={[typeScale.sub, { color: tokens.ink3 }]}>This file is empty.</Text>
      )}
    </ScrollView>
  );
}

/** Previews the live draft, not the saved file, so edits show before they are written. */
export function SvgPreview({ content }: { content: string }) {
  const tokens = useTokens();
  const source = useMemo(() => ({ uri: svgDataUri(content) }), [content]);
  return (
    <View style={[styles.fill, styles.stage, { backgroundColor: tokens.bg1 }]} accessibilityLabel="SVG preview">
      <View style={[styles.canvas, { backgroundColor: tokens.bg0, borderColor: tokens.border }]}>
        <Image source={source} resizeMode="contain" style={styles.image} accessibilityLabel="Rendered SVG" />
      </View>
    </View>
  );
}

export function PdfFrame({ sessionId, path }: { sessionId: string; path: string }) {
  const tokens = useTokens();
  const { baseUrl } = useAuth();
  if (Platform.OS !== "web" || !baseUrl || !supportsDirectDaemonEndpoints(baseUrl)) {
    return (
      <View style={[styles.fill, styles.stage, { backgroundColor: tokens.bg0 }]}>
        <Text style={[typeScale.bodyBold, { color: tokens.ink }]}>PDF preview</Text>
        <Text style={[typeScale.sub, styles.note, { color: tokens.ink3 }]}>
          PDFs open in the desktop and web apps, on a direct connection to the machine that holds the file.
        </Text>
      </View>
    );
  }
  return (
    <View style={[styles.fill, { backgroundColor: tokens.bg1 }]}>
      {React.createElement("iframe", {
        src: workspacePdfUrl(baseUrl, sessionId, path),
        title: `PDF ${path}`,
        style: { flex: 1, width: "100%", height: "100%", border: 0, background: "transparent" },
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, minHeight: 0 },
  prose: { padding: space.space16, gap: space.space8 },
  stage: { alignItems: "center", justifyContent: "center", padding: space.space16, gap: space.space8 },
  canvas: {
    width: "100%",
    height: "100%",
    maxWidth: 900,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 8,
    padding: space.space16,
    overflow: "hidden",
  },
  image: { width: "100%", height: "100%" },
  note: { maxWidth: 360, textAlign: "center" },
});
