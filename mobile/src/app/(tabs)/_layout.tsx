// Bottom tabs: Fleet · Alerts · History · More (BUILD_PLAN §6). Tab badge counts (waiting
// count on Alerts) are explicit Batch 4 polish (BUILD_PLAN §7, W11) — not wired here.
import { Tabs } from "expo-router/js-tabs";
import React from "react";
import { Text, type ColorValue } from "react-native";

import { colors } from "../../lib/theme";

function TabGlyph({ glyph, color }: { glyph: string; color: ColorValue }) {
  return <Text style={{ color, fontSize: 18 }}>{glyph}</Text>;
}

export default function TabsLayout() {
  return (
    <Tabs
      screenOptions={{
        headerStyle: { backgroundColor: colors.panel },
        headerTintColor: colors.ink,
        headerTitleStyle: { color: colors.ink, fontWeight: "700" },
        tabBarStyle: { backgroundColor: colors.panel, borderTopColor: colors.border },
        tabBarActiveTintColor: colors.accent,
        tabBarInactiveTintColor: colors.dim,
      }}
    >
      <Tabs.Screen
        name="index"
        options={{
          title: "Fleet",
          tabBarIcon: ({ color }) => <TabGlyph glyph="⚙" color={color} />,
        }}
      />
      <Tabs.Screen
        name="alerts"
        options={{
          title: "Alerts",
          tabBarIcon: ({ color }) => <TabGlyph glyph="⚠" color={color} />,
        }}
      />
      <Tabs.Screen
        name="history"
        options={{
          title: "History",
          tabBarIcon: ({ color }) => <TabGlyph glyph="◷" color={color} />,
        }}
      />
      <Tabs.Screen
        name="more"
        options={{
          title: "More",
          tabBarIcon: ({ color }) => <TabGlyph glyph="•••" color={color} />,
        }}
      />
    </Tabs>
  );
}
