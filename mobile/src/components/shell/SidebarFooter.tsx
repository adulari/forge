// The desktop sidebar's footer: the usage-pace rings (each carrying its provider's mark), then
// the nav icons (Inbox, Notices, History, Schedules, What's New) and the window row (phone
// pairing, usage dock, Settings, collapse). Schedules and What's New open their panels in place.
import { router, usePathname } from "expo-router";
import { Bell, BellDot, CalendarClock, Gauge, History, PanelLeftClose, Settings2, Smartphone, Sparkles } from "lucide-react-native";
import React, { useEffect, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { useChangelog } from "../../lib/queries";
import { setWhatsNewSeen, useShellPrefs } from "../../lib/shellPrefs";
import { useShortcutLabel } from "../../lib/shortcuts";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, tabularNums, type as typeScale } from "../../theme/typography";
import { providerFamily, providerFamilyLabel } from "../../lib/providerFamily";
import { useClockSeconds } from "../ds/relativeClock";
import { ringSummary, useProviderPace, UsageRing } from "./UsageRing";
import { SchedulesPanel } from "./SchedulesPanel";
import { WhatsNewPanel } from "./WhatsNewPanel";

/** Whether the newest release is one this viewer has not opened What's New for. A viewer who has
 * never opened it is taken to have seen the release they are on, so the dot only ever announces
 * a release that arrived after they started using the app. */
function useWhatsNewUnread(): { unread: boolean; latest: string | null } {
  const { data } = useChangelog(2);
  const prefs = useShellPrefs();
  const latest = data?.find((release) => release.version.toLowerCase() !== "unreleased")?.version ?? null;
  useEffect(() => {
    if (prefs.loaded && prefs.whatsNewSeen == null && latest) setWhatsNewSeen(latest);
  }, [prefs.loaded, prefs.whatsNewSeen, latest]);
  return { unread: latest != null && prefs.whatsNewSeen != null && prefs.whatsNewSeen !== latest, latest };
}

function planName(provider: string): string {
  return providerFamilyLabel(providerFamily(provider));
}

function FooterIcon({
  label,
  hint,
  selected,
  badge,
  onPress,
  children,
}: {
  label: string;
  hint?: string;
  selected?: boolean;
  badge?: string;
  onPress: () => void;
  children: React.ReactNode;
}) {
  const tokens = useTokens();
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityState={{ selected: !!selected }}
      accessibilityLabel={label}
      accessibilityHint={hint}
      style={({ hovered }: { pressed: boolean; hovered?: boolean }) => [styles.icon, (selected || hovered) && { backgroundColor: selected ? tokens.selection : tokens.bg3 }]}
    >
      {children}
      {badge ? <View style={[styles.badge, { backgroundColor: badge, borderColor: tokens.bg1 }]} /> : null}
    </Pressable>
  );
}

export interface SidebarFooterProps {
  onCollapse: () => void;
  onToggleDock?: () => void;
  dockOpen: boolean;
  waitingCount: number;
  noticesUnseen: number;
}

export function SidebarFooter({ onCollapse, onToggleDock, dockOpen, waitingCount, noticesUnseen }: SidebarFooterProps) {
  const tokens = useTokens();
  const pathname = usePathname();
  const usageHint = useShortcutLabel("shell.usage");
  const sidebarHint = useShortcutLabel("shell.sidebar");
  const { rings } = useProviderPace();
  const { unread, latest } = useWhatsNewUnread();
  const [whatsNew, setWhatsNew] = useState(false);
  const [schedules, setSchedules] = useState(false);
  const nowSec = useClockSeconds();

  return (
    <View style={[styles.footer, { borderTopColor: tokens.border }]}>
      {rings.length > 0 ? (
        <Pressable
          onPress={onToggleDock}
          disabled={!onToggleDock}
          accessibilityRole={onToggleDock ? "button" : undefined}
          accessibilityLabel="Toggle usage dock"
          accessibilityHint={usageHint}
          style={styles.usageBlock}
        >
          <Text style={[typeScale.section, styles.usageLabel, { color: tokens.ink3 }]}>Usage</Text>
          <View style={styles.ringRow}>
            {rings.map((ring) => (
              // A bare ring at a low percent reads as a loading spinner, so it carries its number.
              <View key={ring.provider} style={styles.ringItem}>
                <UsageRing
                  pct={ring.pct}
                  size={20}
                  provider={ring.provider}
                  tooltip={ringSummary(ring, nowSec)}
                  accessibilityLabel={ringSummary(ring, nowSec)}
                />
                <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3 }]}>
                  {planName(ring.provider)} {ring.pct}%
                </Text>
              </View>
            ))}
          </View>
        </Pressable>
      ) : null}

      <View style={styles.iconRow}>
        <FooterIcon
          label={waitingCount > 0 ? `Inbox, ${waitingCount} needs you` : "Inbox"}
          selected={pathname === "/inbox"}
          badge={waitingCount > 0 ? tokens.danger : undefined}
          onPress={() => router.push("/inbox")}
        >
          <BellDot size={15} strokeWidth={1.75} color={tokens.ink2} />
        </FooterIcon>
        <FooterIcon
          label={noticesUnseen > 0 ? `Notices, ${noticesUnseen} new` : "Notices"}
          selected={pathname === "/notices"}
          badge={noticesUnseen > 0 ? tokens.accent : undefined}
          onPress={() => router.push("/notices")}
        >
          <Bell size={15} strokeWidth={1.75} color={tokens.ink2} />
        </FooterIcon>
        <FooterIcon label="History" selected={pathname === "/history"} onPress={() => router.push("/history")}>
          <History size={15} strokeWidth={1.75} color={tokens.ink2} />
        </FooterIcon>
        <FooterIcon label="Schedules" onPress={() => setSchedules(true)}>
          <CalendarClock size={15} strokeWidth={1.75} color={tokens.ink2} />
        </FooterIcon>
        <FooterIcon
          label={unread ? "What's new, unread release" : "What's new"}
          badge={unread ? tokens.accent : undefined}
          onPress={() => {
            if (latest) setWhatsNewSeen(latest);
            setWhatsNew(true);
          }}
        >
          <Sparkles size={15} strokeWidth={1.75} color={tokens.ink2} />
        </FooterIcon>
      </View>

      <View style={styles.iconRow}>
        <FooterIcon label="Pair a phone" hint="Opens Forge Anywhere pairing" selected={pathname === "/anywhere/pair"} onPress={() => router.push("/anywhere/pair")}>
          <Smartphone size={15} strokeWidth={1.75} color={tokens.ink2} />
        </FooterIcon>
        <View style={styles.spacer} />
        {/* Always present: the ring row above only renders once a subscription window has been
            observed, and without this the dock had no visible way to open. */}
        {onToggleDock ? (
          <FooterIcon label={dockOpen ? "Hide usage panel" : "Show usage panel"} hint={usageHint} selected={dockOpen} onPress={onToggleDock}>
            <Gauge size={15} strokeWidth={1.75} color={dockOpen ? tokens.ink : tokens.ink2} />
          </FooterIcon>
        ) : null}
        <FooterIcon label="Settings" selected={pathname === "/settings"} onPress={() => router.push("/settings")}>
          <Settings2 size={15} strokeWidth={1.75} color={tokens.ink2} />
        </FooterIcon>
        <FooterIcon label="Collapse sidebar" hint={sidebarHint} onPress={onCollapse}>
          <PanelLeftClose size={15} strokeWidth={1.75} color={tokens.ink2} />
        </FooterIcon>
      </View>

      <WhatsNewPanel visible={whatsNew} onClose={() => setWhatsNew(false)} />
      <SchedulesPanel visible={schedules} onClose={() => setSchedules(false)} />
    </View>
  );
}

const styles = StyleSheet.create({
  footer: { borderTopWidth: StyleSheet.hairlineWidth, paddingTop: space.space8, paddingBottom: space.space4, gap: space.space2 },
  usageBlock: { paddingHorizontal: space.space4, marginBottom: space.space8 },
  usageLabel: { fontFamily: monoFamily.regular, marginBottom: space.space8 },
  ringRow: { flexDirection: "row", flexWrap: "wrap", gap: space.space12, alignItems: "center" },
  ringItem: { flexDirection: "row", gap: space.space4, alignItems: "center" },
  iconRow: { flexDirection: "row", alignItems: "center", gap: space.space4, paddingHorizontal: space.space2 },
  icon: { width: 30, height: 30, alignItems: "center", justifyContent: "center", borderRadius: radii.radius8 },
  badge: { position: "absolute", top: 4, right: 4, width: 6, height: 6, borderRadius: 3, borderWidth: 1 },
  spacer: { flex: 1 },
});
