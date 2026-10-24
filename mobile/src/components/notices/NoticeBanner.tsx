// In-app banner for newly fetched notices: the real title and message the lock-screen push could
// not carry. Top-anchored like ToastHost (which owns generic strings), but pressable — tap opens
// the notice's session, or the notices list when it has none.
import { X } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { bannerCopy, type Notice } from "../../lib/notices/core";
import { isTauri } from "../../lib/platform";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";

export interface NoticeBannerProps {
  notices: readonly Notice[];
  onOpen: () => void;
  onDismiss: () => void;
}

export function NoticeBanner({ notices, onOpen, onDismiss }: NoticeBannerProps) {
  const tokens = useTokens();
  if (notices.length === 0) return null;
  const copy = bannerCopy(notices);
  return (
    <SafeAreaView
      pointerEvents="box-none"
      style={[styles.host, { paddingTop: isTauri ? 44 : 0 }]}
      edges={["top"]}
    >
      <View
        style={[
          styles.card,
          { backgroundColor: tokens.bg2, borderColor: tokens.border },
        ]}
        accessibilityRole="alert"
      >
        <Pressable
          onPress={onOpen}
          style={styles.body}
          accessibilityRole="button"
          accessibilityLabel={`${copy.title}. ${copy.body}`}
          accessibilityHint="Opens the notice"
        >
          <Text style={[typeScale.bodyBold, { color: tokens.ink }]} numberOfLines={1}>
            {copy.title}
          </Text>
          <Text style={[typeScale.sub, { color: tokens.ink2 }]} numberOfLines={3}>
            {copy.body}
          </Text>
        </Pressable>
        <Pressable onPress={onDismiss} accessibilityRole="button" accessibilityLabel="Dismiss notice" hitSlop={8}>
          <X size={16} strokeWidth={1.75} color={tokens.ink3} />
        </Pressable>
      </View>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  host: { position: "absolute", left: 0, right: 0, top: 0, alignItems: "center" },
  card: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: space.space8,
    alignSelf: "stretch",
    maxWidth: 520,
    marginHorizontal: space.space16,
    paddingHorizontal: space.space12,
    paddingVertical: space.space12,
    marginTop: space.space8,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius8,
  },
  body: { flex: 1, gap: space.space4 },
});
