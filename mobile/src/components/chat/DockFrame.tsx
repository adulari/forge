// The shared chrome of a composer dock: a folding header (icon, title, count, and the headline
// while folded) with a menu for the things every dock can do, over the dock's own body. It sits
// directly on the composer, so the two read as one object.
import { ChevronDown, ChevronRight, MoreHorizontal } from "lucide-react-native";
import type { LucideIcon } from "lucide-react-native";
import React, { useRef, useState } from "react";
import { StyleSheet, Text, View, useWindowDimensions } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";
import { type MenuAnchor, type MenuEntry, PopoverMenu } from "../shell/PopoverMenu";
import { MiniPressable } from "./MiniPressable";

export interface DockFrameProps {
  title: string;
  icon: LucideIcon;
  /** Mono readout beside the title: `2/4`, `$0.42`. */
  meta?: string;
  /** One line shown while the dock is folded. */
  headline?: string | null;
  collapsed: boolean;
  onToggle: () => void;
  menu: readonly MenuEntry[];
  testID: string;
  children: React.ReactNode;
}

export function DockFrame({ title, icon: Icon, meta, headline, collapsed, onToggle, menu, testID, children }: DockFrameProps) {
  const tokens = useTokens();
  const { width: windowWidth } = useWindowDimensions();
  const menuRef = useRef<View>(null);
  const [anchor, setAnchor] = useState<MenuAnchor | null>(null);
  const openMenu = () =>
    menuRef.current?.measureInWindow((x, y, width, height) =>
      setAnchor({ top: y + height + 4, right: Math.max(8, windowWidth - x - width) }),
    );

  return (
    <View style={[styles.frame, { backgroundColor: tokens.bg1, borderColor: tokens.border }]} testID={testID}>
      <View style={styles.header}>
        <MiniPressable
          onPress={onToggle}
          accessibilityRole="button"
          accessibilityState={{ expanded: !collapsed }}
          accessibilityLabel={`${collapsed ? "Expand" : "Collapse"} ${title}${meta ? `, ${meta}` : ""}`}
          wash={false}
          style={styles.toggle}
          testID={`${testID}-toggle`}
        >
          {collapsed ? <ChevronRight size={13} strokeWidth={1.75} color={tokens.ink3} /> : <ChevronDown size={13} strokeWidth={1.75} color={tokens.ink3} />}
          <Icon size={13} strokeWidth={1.75} color={tokens.ink3} />
          <Text style={[typeScale.meta, { color: tokens.ink2 }]}>{title}</Text>
          {meta ? <Text style={[styles.meta, { color: tokens.ink3 }]}>{meta}</Text> : null}
          {collapsed && headline ? (
            <Text style={[typeScale.sub, styles.headline, { color: tokens.ink3 }]} numberOfLines={1}>
              {headline}
            </Text>
          ) : (
            <View style={styles.fill} />
          )}
        </MiniPressable>
        <View ref={menuRef} collapsable={false}>
          <MiniPressable
            onPress={openMenu}
            accessibilityRole="button"
            accessibilityLabel={`${title} options`}
            accessibilityState={{ expanded: anchor != null }}
            hitSlop={{ top: 6, bottom: 6, left: 4, right: 4 }}
            style={styles.menuButton}
            testID={`${testID}-menu`}
          >
            <MoreHorizontal size={14} strokeWidth={1.75} color={tokens.ink3} />
          </MiniPressable>
        </View>
      </View>
      {collapsed ? null : children}
      <PopoverMenu anchor={anchor} entries={menu} onClose={() => setAnchor(null)} accessibilityLabel={`${title} options`} width={232} />
    </View>
  );
}

const styles = StyleSheet.create({
  frame: {
    marginHorizontal: space.space12,
    marginBottom: space.space4,
    borderWidth: StyleSheet.hairlineWidth * 2,
    borderRadius: radii.radius8,
    overflow: "hidden",
  },
  header: { flexDirection: "row", alignItems: "center" },
  toggle: {
    flex: 1,
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingLeft: space.space8,
    paddingVertical: space.space4,
    minHeight: 28,
  },
  meta: { fontFamily: monoFamily.regular, fontSize: 11, lineHeight: 15 },
  headline: { flex: 1, minWidth: 0 },
  fill: { flex: 1 },
  menuButton: { paddingHorizontal: space.space8, paddingVertical: space.space4, minHeight: 28, justifyContent: "center" },
});
