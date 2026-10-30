// Desktop/web message actions: a compact popover anchored at the pointer — the mobile
// bottom sheet reads as bugged when it spans a 1440px window for a right-click menu.
// Same actions as MessageActionsSheet; that sheet remains the touch/long-press surface.
import * as Clipboard from "expo-clipboard";
import { Copy, PencilLine, Quote, Undo2 } from "lucide-react-native";
import React from "react";
import { Modal, Pressable, StyleSheet, Text, useWindowDimensions, View } from "react-native";

import { displayMessageText } from "./MessageRow";
import type { HistoryRow } from "../../lib/api";
import { useTokens } from "../../theme/ThemeProvider";
import { depthDark, radii, shadowStyle, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { useToast } from "../ds/ToastHost";
import { useOverlayEscape } from "../../lib/useOverlayEscape";

export interface MessageActionsMenuProps {
  message: HistoryRow | null;
  anchor: { x: number; y: number } | null;
  onClose: () => void;
  onQuote: (text: string) => void;
  onEdit?: (text: string) => void;
  onRewind?: (message: HistoryRow) => void;
  rewindDisabledReason?: string | null;
  rewindWarning?: string | null;
}

const MENU_WIDTH = 220;
const ROW_HEIGHT = 40;
const MENU_MARGIN = 8;
const CAPTION_HEIGHT = 52;

function firstCodeBlock(text: string): string | null {
  const match = text.match(/```[^\n]*\n([\s\S]*?)```/);
  return match ? match[1].replace(/\n$/, "") : null;
}

export function MessageActionsMenu({ message, anchor, onClose, onQuote, onEdit, onRewind, rewindDisabledReason, rewindWarning }: MessageActionsMenuProps) {
  const tokens = useTokens();
  const toast = useToast();
  const { width: windowWidth, height: windowHeight } = useWindowDimensions();
  useOverlayEscape(message != null && anchor != null, onClose);
  if (message == null || anchor == null) return null;

  const text = displayMessageText(message);
  const code = firstCodeBlock(text);
  const editable = message.role === "user" && onEdit != null;

  const rewindable = message.role === "user" && onRewind != null;
  const rows: { label: string; icon: React.ReactNode; run: () => void; disabledReason?: string | null; caption?: string | null }[] = [
    {
      label: "Copy text",
      icon: <Copy size={15} strokeWidth={1.75} color={tokens.ink2} />,
      run: () => {
        void Clipboard.setStringAsync(text).then(() => toast.show("Copied"));
      },
    },
    ...(code !== null
      ? [
          {
            label: "Copy code",
            icon: <Copy size={15} strokeWidth={1.75} color={tokens.ink2} />,
            run: () => {
              void Clipboard.setStringAsync(code).then(() => toast.show("Copied"));
            },
          },
        ]
      : []),
    { label: "Quote", icon: <Quote size={15} strokeWidth={1.75} color={tokens.ink2} />, run: () => onQuote(text) },
    ...(editable
      ? [
          {
            label: "Edit & resend",
            icon: <PencilLine size={15} strokeWidth={1.75} color={tokens.ink2} />,
            run: () => onEdit!(text),
          },
        ]
      : []),
    ...(rewindable
      ? [
          {
            label: "Rewind to here",
            icon: <Undo2 size={15} strokeWidth={1.75} color={tokens.ink2} />,
            run: () => onRewind!(message),
            disabledReason: rewindDisabledReason,
            caption: rewindDisabledReason ?? rewindWarning,
          },
        ]
      : []),
  ];

  const captionRows = rows.filter((row) => row.caption).length;
  const menuHeight = rows.length * ROW_HEIGHT + captionRows * CAPTION_HEIGHT + 2 * MENU_MARGIN;
  const left = Math.max(MENU_MARGIN, Math.min(anchor.x, windowWidth - MENU_WIDTH - MENU_MARGIN));
  const top = Math.max(MENU_MARGIN, Math.min(anchor.y, windowHeight - menuHeight - MENU_MARGIN));

  return (
    <Modal visible transparent animationType="none" onRequestClose={onClose}>
      <Pressable style={styles.backdrop} onPress={onClose} accessibilityLabel="Dismiss message actions">
        <View
          style={[
            styles.menu,
            shadowStyle(depthDark.sheet),
            { left, top, backgroundColor: tokens.bg2, borderColor: tokens.borderStrong },
          ]}
          accessibilityRole="menu"
        >
          {rows.map((row) => (
            <React.Fragment key={row.label}>
              <MenuRow
                label={row.label}
                icon={row.icon}
                disabledReason={row.disabledReason}
                onPress={() => {
                  row.run();
                  onClose();
                }}
              />
              {row.caption ? (
                <Text style={[typeScale.meta, styles.caption, { color: row.disabledReason ? tokens.ink3 : tokens.warn }]}>
                  {row.caption}
                </Text>
              ) : null}
            </React.Fragment>
          ))}
        </View>
      </Pressable>
    </Modal>
  );
}

function MenuRow({ label, icon, onPress, disabledReason }: { label: string; icon: React.ReactNode; onPress: () => void; disabledReason?: string | null }) {
  const disabled = disabledReason != null;
  const tokens = useTokens();
  const [hovered, setHovered] = React.useState(false);
  return (
    <Pressable
      onPress={disabled ? undefined : onPress}
      disabled={disabled}
      {...(disabled ? ({ title: disabledReason } as object) : null)}
      accessibilityHint={disabledReason ?? undefined}
      onHoverIn={() => setHovered(true)}
      onHoverOut={() => setHovered(false)}
      accessibilityRole="menuitem"
      accessibilityLabel={label}
      accessibilityState={{ disabled }}
      style={[styles.menuRow, { backgroundColor: hovered && !disabled ? tokens.bg3 : "transparent", opacity: disabled ? 0.5 : 1 }]}
    >
      {icon}
      <Text style={[typeScale.sub, { color: tokens.ink }]}>{label}</Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  backdrop: { flex: 1 },
  menu: {
    position: "absolute",
    width: MENU_WIDTH,
    paddingVertical: MENU_MARGIN,
    borderRadius: radii.radius12,
    borderWidth: StyleSheet.hairlineWidth,
  },
  caption: { paddingHorizontal: space.space16, paddingBottom: space.space8, minHeight: CAPTION_HEIGHT - ROW_HEIGHT + 20 },
  menuRow: {
    height: ROW_HEIGHT,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space12,
    marginHorizontal: space.space4,
    borderRadius: radii.radius8,
  },
});
