// Hearth web shell — the 52px top bar (web.dc.html): flame + "Forge", nav links with the
// Inbox unread dot, a centered ⌘K search-or-command field, and active-server status. Web only;
// the Tauri desktop shell gets the same affordances from DesktopWindowChrome's 36px bar.
import { usePathname, useRouter } from "expo-router";
import { Flame, Search } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { useAuth } from "../lib/auth";
import { useSessions } from "../lib/queries";
import { usePalette } from "./overlay/CommandPalette";
import { useTokens } from "../theme/ThemeProvider";
import { radii, space } from "../theme/tokens";
import { monoFamily } from "../theme/typography";

const NAV_LINKS = [
  { label: "Fleet", href: "/" },
  { label: "Inbox", href: "/inbox" },
  { label: "History", href: "/history" },
  { label: "Settings", href: "/settings" },
] as const;

export const WEB_TOP_BAR_HEIGHT = 52;

export function WebTopBar() {
  const tokens = useTokens();
  const router = useRouter();
  const pathname = usePathname();
  const palette = usePalette();
  const { servers, activeServerId } = useAuth();
  const { data: sessions } = useSessions();
  const hasWaiting = (sessions ?? []).some((s) => s.waiting);
  const activeServer = servers.find((server) => server.id === activeServerId);

  const isActive = (href: string) =>
    href === "/" ? pathname === "/" || pathname.startsWith("/session") : pathname.startsWith(href);

  return (
    <View style={[styles.bar, { backgroundColor: tokens.bg1, borderBottomColor: tokens.border }]}>
      <Flame size={17} color={tokens.accent} strokeWidth={1.75} />
      <Text style={[styles.brand, { color: tokens.ink }]}>Forge</Text>
      <View style={styles.nav}>
        {NAV_LINKS.map(({ label, href }) => (
          <Pressable
            key={href}
            onPress={() => router.navigate(href)}
            accessibilityRole="link"
            accessibilityLabel={label === "Inbox" && hasWaiting ? "Inbox, needs you" : label}
          >
            <View>
              <Text style={[styles.navLink, { color: isActive(href) ? tokens.ink : tokens.ink3 }]}>{label}</Text>
              {label === "Inbox" && hasWaiting ? (
                <View style={[styles.dot, { backgroundColor: tokens.danger }]} />
              ) : null}
            </View>
          </Pressable>
        ))}
      </View>
      <View style={styles.spacer} />
      <Pressable
        onPress={() => palette.open()}
        accessibilityRole="button"
        accessibilityLabel="Search or command"
        style={[styles.search, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}
      >
        <Search size={13} color={tokens.ink4} strokeWidth={2} />
        <Text style={[styles.searchHint, { color: tokens.ink4 }]}>search or command</Text>
        <Text style={[styles.kbd, { color: tokens.ink4, borderColor: tokens.border }]}>⌘K</Text>
      </Pressable>
      <View style={styles.spacer} />
      <View style={styles.server}>
        <View style={[styles.serverDot, { backgroundColor: tokens.success }]} />
        <Text style={[styles.serverLabel, { color: tokens.ink4 }]}>
          {activeServer ? `${activeServer.name} · ok` : "no server"}
        </Text>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  bar: {
    height: WEB_TOP_BAR_HEIGHT,
    flexShrink: 0,
    borderBottomWidth: StyleSheet.hairlineWidth,
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: 20,
    gap: space.space12,
  },
  brand: { fontSize: 14, fontWeight: "700", letterSpacing: -0.2 },
  nav: { flexDirection: "row", alignItems: "center", gap: 20, marginLeft: space.space24 },
  navLink: { fontSize: 13, fontWeight: "600" },
  dot: { position: "absolute", top: -2, right: -7, width: 5, height: 5, borderRadius: 3 },
  spacer: { flex: 1 },
  search: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    height: 32,
    paddingHorizontal: space.space12,
    borderRadius: radii.radius8,
    borderWidth: 1,
    width: 360,
  },
  searchHint: { flex: 1, fontSize: 12 },
  kbd: {
    fontFamily: monoFamily.regular,
    fontSize: 10,
    borderWidth: 1,
    borderRadius: radii.radius4,
    paddingHorizontal: 5,
    paddingVertical: 1,
  },
  server: { flexDirection: "row", alignItems: "center", gap: 7 },
  serverDot: { width: 6, height: 6, borderRadius: 3 },
  serverLabel: { fontFamily: monoFamily.regular, fontSize: 11 },
});
