// Machined web shell — the 40px top bar (W Main, docs/design/machined INVENTORY.md
// L33-43): flame + "Forge", underline tabs (Fleet/Inbox badge/Floor/History/
// Settings), and a right-hand cluster of host status chip + combined usage ring
// (doubles as the ⌘U dock toggle) + a compact ⌘K chip. Web only; the Tauri desktop
// shell gets the equivalent affordances from DesktopWindowChrome's 36px bar.
import { usePathname, useRouter } from "expo-router";
import { Flame, Search } from "lucide-react-native";
import React from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { useAuth } from "../lib/auth";
import { useSessions } from "../lib/queries";
import { usePalette } from "./overlay/CommandPalette";
import { useProviderPace, UsageRing } from "./shell/UsageRing";
import { useTokens } from "../theme/ThemeProvider";
import { radii, space } from "../theme/tokens";
import { monoFamily } from "../theme/typography";

const NAV_LINKS = [
  { label: "Fleet", href: "/" },
  { label: "Inbox", href: "/inbox" },
  { label: "Floor", href: "/floor" },
  { label: "History", href: "/history" },
  { label: "Settings", href: "/settings" },
] as const;

export const WEB_TOP_BAR_HEIGHT = 40;

export interface WebTopBarProps {
  /** Doubles the combined usage ring as the ⌘U dock toggle affordance — see
   * Sidebar/IconRail's identical ring-row treatment. */
  onToggleDock?: () => void;
  dockOpen?: boolean;
}

export function WebTopBar({ onToggleDock, dockOpen }: WebTopBarProps) {
  const tokens = useTokens();
  const router = useRouter();
  const pathname = usePathname();
  const palette = usePalette();
  const { servers, activeServerId } = useAuth();
  const { data: sessions } = useSessions();
  const waitingCount = (sessions ?? []).filter((s) => s.waiting).length;
  const activeServer = servers.find((server) => server.id === activeServerId);
  const { combinedPct } = useProviderPace();

  const isActive = (href: string) =>
    href === "/" ? pathname === "/" || pathname.startsWith("/session") : pathname.startsWith(href);

  return (
    <View style={[styles.bar, { backgroundColor: tokens.bg1, borderBottomColor: tokens.border }]}>
      <Flame size={14} color={tokens.accent} strokeWidth={1.75} />
      <Text style={[styles.brand, { color: tokens.ink }]}>Forge</Text>
      <View style={styles.nav}>
        {NAV_LINKS.map(({ label, href }) => {
          const active = isActive(href);
          return (
            <Pressable
              key={href}
              onPress={() => router.navigate(href)}
              accessibilityRole="link"
              accessibilityLabel={label === "Inbox" && waitingCount > 0 ? `Inbox, ${waitingCount} needs you` : label}
              style={[styles.navLinkWrap, active && { borderBottomColor: tokens.accent }]}
            >
              <Text style={[styles.navLink, { color: active ? tokens.ink : tokens.ink3 }]}>{label}</Text>
              {label === "Inbox" && waitingCount > 0 ? (
                <View style={[styles.badge, { backgroundColor: tokens.danger }]}>
                  <Text style={[styles.badgeText, { color: tokens.onAccent }]} numberOfLines={1}>
                    {waitingCount}
                  </Text>
                </View>
              ) : null}
            </Pressable>
          );
        })}
      </View>
      <View style={styles.spacer} />
      <View style={styles.server}>
        <View style={[styles.serverDot, { backgroundColor: tokens.success }]} />
        <Text style={[styles.serverLabel, { color: tokens.ink4 }]}>
          {activeServer ? `${activeServer.name} · ok` : "no server"}
        </Text>
      </View>
      {combinedPct != null ? (
        <Pressable
          onPress={onToggleDock}
          disabled={!onToggleDock}
          accessibilityRole={onToggleDock ? "button" : undefined}
          accessibilityLabel={`Combined usage ${combinedPct}%${dockOpen ? ", close usage dock" : ", open usage dock"}`}
          accessibilityHint="Command U"
          style={[styles.ringButton, dockOpen && { backgroundColor: tokens.bg3 }]}
        >
          <UsageRing pct={combinedPct} size={15} />
        </Pressable>
      ) : null}
      <Pressable
        onPress={() => palette.open("default")}
        accessibilityRole="button"
        accessibilityLabel="Search or command"
        style={[styles.kbdChip, { borderColor: tokens.border }]}
      >
        <Search size={11} color={tokens.ink4} strokeWidth={2} />
        <Text style={[styles.kbd, { color: tokens.ink4 }]}>⌘K</Text>
      </Pressable>
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
    paddingHorizontal: space.space16,
    gap: space.space8,
  },
  brand: { fontSize: 12.5, fontWeight: "700", letterSpacing: -0.2 },
  nav: { flexDirection: "row", alignItems: "center", height: "100%", marginLeft: space.space16 },
  navLinkWrap: {
    height: "100%",
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: space.space8,
    borderBottomWidth: 2,
    borderBottomColor: "transparent",
  },
  navLink: { fontSize: 11.5, fontWeight: "600" },
  badge: { marginLeft: 5, minWidth: 13, height: 13, borderRadius: 7, alignItems: "center", justifyContent: "center", paddingHorizontal: 3 },
  badgeText: { fontFamily: monoFamily.regular, fontSize: 8.5, fontWeight: "700" },
  spacer: { flex: 1 },
  server: { flexDirection: "row", alignItems: "center", gap: 6 },
  serverDot: { width: 5, height: 5, borderRadius: 2.5 },
  serverLabel: { fontFamily: monoFamily.regular, fontSize: 10.5 },
  ringButton: { marginLeft: space.space12, width: 26, height: 26, alignItems: "center", justifyContent: "center", borderRadius: radii.radius8 },
  kbdChip: {
    marginLeft: space.space8,
    flexDirection: "row",
    alignItems: "center",
    gap: 5,
    borderWidth: 1,
    borderRadius: radii.radius4,
    paddingHorizontal: 6,
    paddingVertical: 2,
  },
  kbd: { fontFamily: monoFamily.regular, fontSize: 10 },
});
