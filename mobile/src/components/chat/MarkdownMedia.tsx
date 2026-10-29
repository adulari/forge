// Images inside chat markdown, and the one viewer a message's images and image-path chips share.
// An image in a reply is a project file (or an evidence screenshot), fetched through the session's
// daemon; a remote URL is never fetched on its own, because an agent's output should not be able to
// make the app phone out. It shows as a link instead.
import { ImageOff, ExternalLink } from "lucide-react-native";
import React, { createContext, useCallback, useContext, useMemo, useState } from "react";
import { Image, Linking, Pressable, StyleSheet, Text, View } from "react-native";

import type { ViewerImage } from "../../lib/imageTypes";
import { safeExternalHref } from "../../lib/linkSafety";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";
import { ImageViewer } from "../ds/ImageViewer";
import { usePathActions, type PathActions } from "./pathActions";
import { useImageRatio } from "./useImageRatio";

interface MarkdownMedia {
  /** What a click on a path does, for the session that owns this markdown; null outside one. */
  actions: PathActions | null;
  openImage: (image: ViewerImage) => void;
}

const MarkdownMediaContext = createContext<MarkdownMedia | null>(null);

export function useMarkdownMedia(): MarkdownMedia | null {
  return useContext(MarkdownMediaContext);
}

/** Wraps a markdown tree: resolves the session's path actions once for all of its chips and owns
 * the viewer its images and chips open. */
export function MarkdownMediaProvider({ children }: { children: React.ReactNode }) {
  const actions = usePathActions();
  const [current, setCurrent] = useState<ViewerImage | null>(null);
  const openImage = useCallback((image: ViewerImage) => setCurrent(image), []);
  const value = useMemo(() => ({ actions, openImage }), [actions, openImage]);
  const images = useMemo(() => (current ? [current] : []), [current]);
  return (
    <MarkdownMediaContext.Provider value={value}>
      {children}
      {current ? <ImageViewer visible images={images} onClose={() => setCurrent(null)} /> : null}
    </MarkdownMediaContext.Provider>
  );
}

const THUMB_WIDTH = 960;
const MAX_HEIGHT = 320;

function isLocal(src: string): boolean {
  return !/^[a-z][a-z0-9+.-]*:/i.test(src) || src.startsWith("file://");
}

export function MarkdownImage({ alt, src }: { alt: string; src: string }) {
  const tokens = useTokens();
  const actions = useMarkdownMedia()?.actions ?? null;
  const [failed, setFailed] = useState(false);
  const name = (alt || src.slice(src.lastIndexOf("/") + 1)).trim();

  if (!isLocal(src) || !actions) {
    const href = safeExternalHref(src);
    return (
      <Pressable
        onPress={href ? () => void Linking.openURL(href).catch(() => {}) : undefined}
        disabled={!href}
        accessibilityRole="link"
        accessibilityLabel={`Open image link ${name}`}
        style={[styles.note, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}
      >
        <ExternalLink size={14} color={tokens.ink3} />
        <Text style={[type.meta, styles.noteText, { color: tokens.ink2 }]} numberOfLines={1}>
          {name || "image"}
          {href ? ` · ${hostOf(href)}` : ""}
        </Text>
      </Pressable>
    );
  }

  if (failed) {
    return (
      <View style={[styles.note, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}>
        <ImageOff size={14} color={tokens.ink3} />
        <Text style={[type.meta, styles.noteText, { color: tokens.ink3, fontFamily: monoFamily.regular }]} numberOfLines={1}>
          {src}
        </Text>
      </View>
    );
  }

  return <LocalImage alt={alt} src={src} name={name} onFail={() => setFailed(true)} />;
}

function LocalImage({ alt, src, name, onFail }: { alt: string; src: string; name: string; onFail: () => void }) {
  const tokens = useTokens();
  const media = useMarkdownMedia();
  const actions = media?.actions ?? null;
  const thumb = actions ? actions.imageUrl(src, THUMB_WIDTH) : "";
  const ratio = useImageRatio(thumb, 16 / 9);
  if (!actions) return null;
  const open = () => media?.openImage({ uri: actions.imageUrl(src), name, caption: alt && alt !== name ? alt : src });
  return (
    <Pressable
      onPress={open}
      accessibilityRole="button"
      accessibilityLabel={`${name}, open image`}
      style={({ pressed }) => [styles.frame, { borderColor: tokens.border, backgroundColor: tokens.bg2, opacity: pressed ? 0.85 : 1 }]}
    >
      <Image
        source={{ uri: thumb }}
        resizeMode="contain"
        onError={onFail}
        accessibilityLabel={alt || name}
        style={[styles.image, { aspectRatio: ratio, width: Math.min(560, MAX_HEIGHT * ratio) }]}
      />
    </Pressable>
  );
}

function hostOf(href: string): string {
  try {
    return new URL(href).host;
  } catch {
    return href;
  }
}

const styles = StyleSheet.create({
  frame: {
    alignSelf: "flex-start",
    maxWidth: "100%",
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius8,
    overflow: "hidden",
  },
  image: { maxWidth: "100%", maxHeight: MAX_HEIGHT },
  note: {
    alignSelf: "flex-start",
    maxWidth: "100%",
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius8,
    paddingHorizontal: space.space8,
    paddingVertical: space.space4,
  },
  noteText: { flexShrink: 1 },
});
