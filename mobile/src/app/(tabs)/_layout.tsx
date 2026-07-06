// Bottom tabs: Fleet · Alerts · History · More (BUILD_PLAN §6). The Alerts tab carries a
// numeric badge = count of sessions currently `waiting` (from useSessions(), the same
// polled list the Fleet/Alerts tabs render — no extra fetch).
import { Tabs } from "expo-router";
import React from "react";
import { Text, type ColorValue } from "react-native";

import { useSessions } from "../../lib/queries";
import { colors } from "../../lib/theme";

function TabGlyph({ glyph, color }: { glyph: string; color: ColorValue }) {
  return <Text style={{ color, fontSize: 18 }}>{glyph}</Text>;
}

export default function TabsLayout() {
  const { data: sessions } = useSessions();
  const waitingCount = sessions?.filter((s) => s.waiting).length ?? 0;

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
          tabBarIcon: ({ color }: { color: ColorValue }) => (
            <TabGlyph glyph="⚙" color={color} />
          ),
        }}
      />
      <Tabs.Screen
        name="alerts"
        options={{
          title: "Alerts",
          tabBarIcon: ({ color }: { color: ColorValue }) => (
            <TabGlyph glyph="⚠" color={color} />
          ),
          tabBarBadge: waitingCount > 0 ? waitingCount : undefined,
          tabBarBadgeStyle: { backgroundColor: colors.no, color: colors.panel },
        }}
      />
      <Tabs.Screen
        name="history"
        options={{
          title: "History",
          tabBarIcon: ({ color }: { color: ColorValue }) => (
            <TabGlyph glyph="◷" color={color} />
          ),
        }}
      />
      <Tabs.Screen
        name="more"
        options={{
          title: "More",
          tabBarIcon: ({ color }: { color: ColorValue }) => (
            <TabGlyph glyph="•••" color={color} />
          ),
        }}
      />
    </Tabs>
  );
}
