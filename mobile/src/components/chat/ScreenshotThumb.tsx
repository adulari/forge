// The picture a browser or device screenshot tool call took, shown under its row. It comes from
// the evidence folder through the daemon; a screenshot that is not there (written to a temp file
// outside any session) simply shows nothing.
import React, { useMemo, useState } from "react";
import { Image, Pressable, StyleSheet } from "react-native";

import type { ViewerImage } from "../../lib/imageTypes";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { useImageViewer } from "../ds/ImageViewer";
import { usePathActions } from "./pathActions";
import { useImageRatio } from "./useImageRatio";

const THUMB_WIDTH = 480;

export function ScreenshotThumb({ path }: { path: string }) {
  const tokens = useTokens();
  const actions = usePathActions();
  const [failed, setFailed] = useState(false);
  const name = path.slice(path.lastIndexOf("/") + 1);
  const images = useMemo<ViewerImage[]>(
    () => (actions ? [{ uri: actions.imageUrl(path), name, caption: "Screenshot" }] : []),
    [actions, name, path],
  );
  const viewer = useImageViewer(images);
  const thumb = actions ? actions.imageUrl(path, THUMB_WIDTH) : "";
  const ratio = useImageRatio(thumb, 16 / 10);
  if (!actions || failed) return null;

  return (
    <>
      <Pressable
        onPress={() => viewer.open(0)}
        accessibilityRole="button"
        accessibilityLabel={`Screenshot ${name}, open image`}
        style={({ pressed }) => [styles.frame, { borderColor: tokens.border, backgroundColor: tokens.bg2, opacity: pressed ? 0.85 : 1 }]}
      >
        <Image
          source={{ uri: thumb }}
          resizeMode="contain"
          onError={() => setFailed(true)}
          style={[styles.image, { aspectRatio: ratio, width: Math.min(240, 160 * ratio) }]}
        />
      </Pressable>
      {viewer.element}
    </>
  );
}

const styles = StyleSheet.create({
  frame: {
    alignSelf: "flex-start",
    marginLeft: space.space24,
    marginVertical: space.space4,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius8,
    overflow: "hidden",
  },
  image: { maxWidth: "100%", maxHeight: 160 },
});
