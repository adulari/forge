// A repo path in chat as a chip: a dot in the file type's colour, the path as written, and the
// line when there is one. Click or Enter opens the file in the Files dock at that line; an image
// path opens the image viewer instead. Where no session owns the chat it is plain mono text.
import React, { useCallback } from "react";
import { Platform, Text, type TextStyle } from "react-native";

import { fileKind, isImagePath, kindHue, type PathRef } from "../../lib/pathRefs";
import { shortenPath } from "../../lib/pathDisplay";
import { useTheme } from "../../theme/ThemeProvider";
import { monoFamily, type } from "../../theme/typography";
import { useMarkdownMedia } from "./MarkdownMedia";

const WEB = Platform.OS === "web";

/** A colour for a file type, tuned per theme so the dot reads on both. */
export function kindColor(path: string, scheme: "dark" | "light", fallback: string): string {
  const hue = kindHue(fileKind(path));
  if (hue == null) return fallback;
  return scheme === "dark" ? `hsl(${hue}, 62%, 66%)` : `hsl(${hue}, 58%, 38%)`;
}

export function PathChip({ pathRef, text }: { pathRef: PathRef; text: string }) {
  const { tokens, scheme } = useTheme();
  const media = useMarkdownMedia();
  const actions = media?.actions ?? null;
  const image = isImagePath(pathRef.path);
  const shown = shortenPath(text, 52);

  const open = useCallback(() => {
    if (!actions) return;
    if (image && media) {
      media.openImage({ uri: actions.imageUrl(pathRef.path), name: pathRef.path.slice(pathRef.path.lastIndexOf("/") + 1), caption: pathRef.path });
    } else {
      actions.openFile(pathRef);
    }
  }, [actions, image, media, pathRef]);

  const base: TextStyle = {
    ...type.codeSmall,
    fontFamily: monoFamily.regular,
    backgroundColor: tokens.bg3,
    color: tokens.ink,
    borderRadius: 4,
    paddingHorizontal: 4,
  };
  const dot = <Text style={{ color: kindColor(pathRef.path, scheme, tokens.ink3) }}>{"●\u00A0"}</Text>;

  if (!actions) {
    return <Text style={base}>{shown}</Text>;
  }

  const keyboard = WEB
    ? ({
        tabIndex: 0,
        onKeyDown: (event: { key: string; preventDefault: () => void }) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            open();
          }
        },
      } as Record<string, unknown>)
    : null;

  return (
    <Text
      accessibilityRole="link"
      accessibilityLabel={`${image ? "View image" : "Open file"} ${pathRef.path}${pathRef.line ? ` at line ${pathRef.line}` : ""}`}
      onPress={open}
      style={[base, WEB ? ({ cursor: "pointer" } as TextStyle) : null]}
      {...keyboard}
    >
      {dot}
      {shown}
    </Text>
  );
}
