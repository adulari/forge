// BUILD_ORDER Batch 1 — dev-only gallery route. Shows every ds/ component in
// every state, both themes, at whatever breakpoint the window currently is.
// Not part of the shipped navigation graph (T2.1 wires the real tab/stack
// layout); this route exists purely for visual QA during B1.
//
// ControlsGallery/StatusGallery/ContentGallery are authored by parallel T1.1/
// T1.2/T1.4 workers in this same batch — if one isn't present yet at tsc time
// that's an expected transient import error per BUILD_ORDER T1.3, resolved
// once all four batch tasks land.
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { Screen } from "../components/ds/Screen";
import ControlsGallery from "../components/ds/gallery/controls";
import ContainersGallery from "../components/ds/gallery/containers";
import ContentGallery from "../components/ds/gallery/content";
import StatusGallery from "../components/ds/gallery/status";
import { ToastHost } from "../components/ds/ToastHost";
import { useTheme, useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";
import { useBreakpoint } from "../theme/useBreakpoint";

function GalleryBody() {
  const tokens = useTokens();
  const { preference, scheme, setScheme } = useTheme();
  const { bp, width } = useBreakpoint();

  const cycleTheme = () => {
    const next = preference === "system" ? "light" : preference === "light" ? "dark" : "system";
    setScheme(next);
  };

  return (
    <Screen scroll>
      <View style={styles.header}>
        <Text style={[type.title, { color: tokens.ink }]}>Design system gallery</Text>
        <Text style={[type.meta, { color: tokens.ink3 }]}>
          breakpoint: {bp} ({Math.round(width)}pt) · theme preference: {preference} (resolved {scheme})
        </Text>
        <Pressable
          onPress={cycleTheme}
          accessibilityRole="button"
          accessibilityLabel={`Theme preference: ${preference}. Tap to cycle light, dark, system.`}
          style={[styles.themeToggle, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}
        >
          <Text style={[type.bodyBold, { color: tokens.accent }]}>theme: {preference}</Text>
        </Pressable>
      </View>

      <ControlsGallery />
      <StatusGallery />
      <ContainersGallery />
      <ContentGallery />
    </Screen>
  );
}

export default function GalleryRoute() {
  return (
    <ToastHost>
      <GalleryBody />
    </ToastHost>
  );
}

const styles = StyleSheet.create({
  header: { gap: space.space8, paddingVertical: space.space16 },
  themeToggle: {
    alignSelf: "flex-start",
    paddingHorizontal: space.space16,
    minHeight: 44,
    justifyContent: "center",
    paddingVertical: space.space8,
    borderRadius: 8,
    borderWidth: StyleSheet.hairlineWidth,
  },
});
