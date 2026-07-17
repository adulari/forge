// Hooks (Native Features pack — "NF Hooks" / "NF Desktop Hooks"). Event-badged,
// hairline-separated read-only rows: each hook is [event badge] + mono command +
// matcher / timeout meta, with a "cc" pill when the hook is Claude-compatible.
//
// Wire scope (lib/api.ts HookRow = {event, matcher, command, timeout_secs, cc_compat}):
// the prototype's per-hook enable TOGGLE, the "last: ok/blocked · <time>" LAST-RUN line,
// and the interactive "Add hook" form all write/observe state the REST contract does not
// carry, so they are omitted (see report). Hooks are configured in the Forge config file
// under `[[hooks]]`; the desktop side pane and the compact footer say exactly that instead
// of offering a form that couldn't persist.
import { Zap } from "lucide-react-native";
import React, { useMemo, useState } from "react";
import { RefreshControl, StyleSheet, Text, View, type ViewStyle } from "react-native";

import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { BackLink } from "../components/ds/BackLink";
import { EmptyState } from "../components/ds/EmptyState";
import { Screen } from "../components/ds/Screen";
import { SearchField } from "../components/ds/SearchField";
import { type HookRow } from "../lib/api";
import { useHooks } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { type ColorTokens, radii, space } from "../theme/tokens";
import { monoFamily, type } from "../theme/typography";
import { useBreakpoint } from "../theme/useBreakpoint";
import { SettingsShell } from "./(tabs)/settings";

// Derive a translucent surface from a token hex (the info event badge has no `*Bg`
// token — successBg/dangerBg/warnBg exist, infoBg does not — so it is tinted here from
// tokens.info exactly as the prototype's rgba(79,208,217,.12) chip; still token-derived,
// never a raw literal).
function tint(hex: string, alpha: number): string {
  const h = hex.replace("#", "");
  const r = parseInt(h.slice(0, 2), 16);
  const g = parseInt(h.slice(2, 4), 16);
  const b = parseInt(h.slice(4, 6), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

type EventKind = "info" | "danger" | "neutral";

// Handoff pattern 8: POST-EDIT-ish → info, PRE-COMMAND-ish → danger, session-end →
// neutral. The wire `event` is a real Forge/Claude-Code hook name (PostToolUse,
// PreToolUse, SessionEnd, Stop, UserPromptSubmit, …); classify by substring.
function eventKind(event: string): EventKind {
  const e = event.toLowerCase();
  if (/(end|stop|finish|submit|notif)/.test(e)) return "neutral";
  if (/(pre|before)/.test(e)) return "danger";
  if (/(post|after|edit|write)/.test(e)) return "info";
  return "neutral";
}

function eventColors(kind: EventKind, tokens: ColorTokens): { bg: string; ink: string } {
  switch (kind) {
    case "danger":
      return { bg: tokens.dangerBg, ink: tokens.danger };
    case "info":
      return { bg: tint(tokens.info, 0.12), ink: tokens.info };
    case "neutral":
    default:
      return { bg: tokens.bg3, ink: tokens.ink2 };
  }
}

function EventBadge({ event }: { event: string }) {
  const tokens = useTokens();
  const { bg, ink } = eventColors(eventKind(event), tokens);
  return (
    <View style={[styles.eventBadge, { backgroundColor: bg }]} accessibilityRole="text" accessibilityLabel={`${event} event`}>
      <Text style={[styles.eventBadgeText, { color: ink }]} numberOfLines={1}>
        {event}
      </Text>
    </View>
  );
}

function HookRowItem({ hook, showSeparator }: { hook: HookRow; showSeparator: boolean }) {
  const tokens = useTokens();
  const matcherPart = hook.matcher ? `matcher ${hook.matcher}` : "all events";
  return (
    <View style={[styles.hook, showSeparator ? { borderBottomColor: tokens.hairline, borderBottomWidth: StyleSheet.hairlineWidth } : null]}>
      <View style={styles.row}>
        <EventBadge event={hook.event} />
        <Text style={[styles.command, { color: tokens.ink }]} numberOfLines={1}>
          {hook.command}
        </Text>
        {hook.cc_compat ? (
          <View style={[styles.ccBadge, { backgroundColor: tokens.selection }]} accessibilityRole="text" accessibilityLabel="Claude Code compatible">
            <Text style={[styles.ccBadgeText, { color: tokens.accent }]}>cc</Text>
          </View>
        ) : null}
      </View>
      <Text style={[type.monoMeta, { color: tokens.ink4 }]} numberOfLines={1}>
        {`${matcherPart} · ${hook.timeout_secs}s timeout`}
      </Text>
    </View>
  );
}

function ConfigHint({ card }: { card: boolean }) {
  const tokens = useTokens();
  const wrapStyle: ViewStyle = card
    ? { backgroundColor: tokens.bg2, borderColor: tokens.border, borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius16, padding: space.space16, gap: space.space8 }
    : { marginTop: space.space16, gap: space.space4 };
  return (
    <View style={wrapStyle}>
      <Text style={[type.section, { color: tokens.ink4 }]}>Configuring hooks</Text>
      <Text style={[type.sub, { color: tokens.ink3 }]}>
        Hooks run around session and tool events. Add or edit them in your Forge config under{" "}
        <Text style={{ fontFamily: monoFamily.regular, color: tokens.ink2 }}>[[hooks]]</Text>.
      </Text>
    </View>
  );
}

function HooksScreenBody() {
  const tokens = useTokens();
  const { isExpanded } = useBreakpoint();
  const query = useHooks();
  const [search, setSearch] = useState("");
  const needle = search.trim().toLocaleLowerCase();
  const hooks = useMemo(
    () => (query.data ?? []).filter((hook) => !needle || `${hook.event} ${hook.matcher ?? ""} ${hook.command}`.toLocaleLowerCase().includes(needle)),
    [query.data, needle],
  );

  const list =
    query.isError && !query.data ? (
      <Text style={[type.body, { color: tokens.danger }]}>Could not load hooks. Pull to retry.</Text>
    ) : !query.isLoading && hooks.length === 0 ? (
      <EmptyState icon={Zap} message={search ? "No hooks match that search." : "No hooks configured."} />
    ) : (
      <View>
        {hooks.map((hook, index) => (
          <HookRowItem key={`${hook.event}-${hook.command}-${index}`} hook={hook} showSeparator={index < hooks.length - 1} />
        ))}
      </View>
    );

  return (
    <Screen scroll refreshControl={<RefreshControl refreshing={query.isFetching} onRefresh={() => void query.refetch()} />} contentContainerStyle={styles.content}>
      <BackLink />
      <Text style={[type.title, { color: tokens.ink }]}>Hooks</Text>
      <Text style={[type.sub, { color: tokens.ink3 }]}>Automations on session and tool events.</Text>
      <SearchField value={search} onChangeText={setSearch} placeholder="Search hooks" accessibilityLabel="Search hooks" />
      {isExpanded ? (
        <View style={styles.twoCol}>
          <View style={styles.colMain}>{list}</View>
          <View style={styles.colSide}>
            <ConfigHint card />
          </View>
        </View>
      ) : (
        <>
          {list}
          <ConfigHint card={false} />
        </>
      )}
    </Screen>
  );
}

export default function HooksScreen() {
  return (
    <DesktopDrillDown>
      <SettingsShell active="hooks">
        <HooksScreenBody />
      </SettingsShell>
    </DesktopDrillDown>
  );
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 },
  twoCol: { flexDirection: "row", gap: space.space32, alignItems: "flex-start" },
  colMain: { flex: 1.1, minWidth: 0 },
  colSide: { flex: 0.9, minWidth: 0 },
  hook: { paddingVertical: space.space12, gap: space.space4 },
  row: { flexDirection: "row", gap: space.space8, alignItems: "center" },
  command: { flex: 1, fontSize: 13, fontFamily: monoFamily.bold },
  eventBadge: { alignSelf: "flex-start", borderRadius: radii.radius4, paddingHorizontal: 7, paddingVertical: 2 },
  eventBadgeText: { fontSize: 10, fontWeight: "700", letterSpacing: 0.3 },
  ccBadge: { borderRadius: radii.radiusPill, paddingHorizontal: space.space8, paddingVertical: 2 },
  ccBadgeText: { fontSize: 10, fontWeight: "700", letterSpacing: 0.3 },
});
