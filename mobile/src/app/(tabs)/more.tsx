// More — search-first launcher (BUILD_PLAN §6 "More", Batch 4 W11). A single SearchInput
// filters across everything the app can do: jump to a live/past session, connection actions
// (Settings, Re-pair, Forget), and About info. Sections are collapsible; when the query is
// empty a short "Recent" list (waiting/busy sessions first, server order) surfaces above them
// so the most useful jump is always one tap away (UI_RULES.md quality bar #34).
import { useQueryClient } from "@tanstack/react-query";
import Constants from "expo-constants";
import { router } from "expo-router";
import React, { useCallback, useMemo, useState } from "react";
import { Alert, Linking, Pressable, Text, View } from "react-native";

import {
  Badge,
  EmptyState,
  EntranceView,
  ListRow,
  Loading,
  Screen,
  SearchInput,
  StatusDot,
  type StatusDotState,
} from "../../components/ui";
import { useAuth } from "../../lib/auth";
import { usePastSessions, useSessions } from "../../lib/queries";

// Verified against README.md badges — not fabricated.
const FORGE_REPO_URL = "https://github.com/Adulari/forge";
// Session section is a quick-jump, not exhaustive search (History tab owns that) — bounded
// so a ScrollView-based launcher never renders an unbounded list (UI_RULES.md #7).
const MAX_SESSION_ITEMS = 50;
const RECENT_COUNT = 3;

interface LauncherItem {
  id: string;
  title: string;
  subtitle?: string;
  right?: React.ReactNode;
  keywords: string;
  onPress?: () => void;
}

interface LauncherSection {
  key: string;
  label: string;
  items: LauncherItem[];
}

function sectionWellClass(): string {
  return "bg-panel border border-borderSoft rounded-md overflow-hidden";
}

export default function MoreScreen() {
  const { host, forget } = useAuth();
  const queryClient = useQueryClient();
  const sessionsQuery = useSessions();
  const pastQuery = usePastSessions();

  const [query, setQuery] = useState("");
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});

  const toggleSection = useCallback((key: string) => {
    setCollapsed((prev) => ({ ...prev, [key]: !prev[key] }));
  }, []);

  const onRepair = useCallback(() => {
    Alert.alert(
      "Re-pair with a different server?",
      "Clears the current pairing so you can paste or scan a new connect URL.",
      [
        { text: "Cancel", style: "cancel" },
        {
          text: "Re-pair",
          onPress: async () => {
            await forget();
            queryClient.clear();
            router.replace("/connect");
          },
        },
      ],
    );
  }, [forget, queryClient]);

  const onForget = useCallback(() => {
    Alert.alert(
      "Forget this server?",
      "Removes the stored connect URL and clears cached data. You'll need to pair again to use Forge.",
      [
        { text: "Cancel", style: "cancel" },
        {
          text: "Forget",
          style: "destructive",
          onPress: async () => {
            await forget();
            queryClient.clear();
            router.replace("/connect");
          },
        },
      ],
    );
  }, [forget, queryClient]);

  const liveSessions = sessionsQuery.data ?? [];
  const pastSessions = useMemo(() => pastQuery.data?.pages.flat() ?? [], [pastQuery.data]);

  const liveItems = useMemo<LauncherItem[]>(
    () =>
      liveSessions.map((row) => {
        const state: StatusDotState = row.waiting ? "waiting" : row.busy ? "busy" : "idle";
        const title = row.title || `#${row.id.slice(0, 8)}`;
        return {
          id: `live:${row.id}`,
          title,
          subtitle: row.cwd,
          right: <StatusDot state={state} />,
          keywords: `${title} ${row.cwd}`.toLowerCase(),
          onPress: () => router.push(`/session/${row.id}`),
        };
      }),
    // Already server-sorted waiting-first, then newest (BUILD_PLAN §1.2) — preserve that order.
    [liveSessions],
  );

  const pastItems = useMemo<LauncherItem[]>(
    () =>
      pastSessions.map((row) => {
        const title = row.title || `#${row.id.slice(0, 8)}`;
        return {
          id: `past:${row.id}`,
          title,
          subtitle: row.cwd,
          right: <Badge label={row.archived ? "ARCHIVED" : "PAST"} tone="default" />,
          keywords: `${title} ${row.cwd}`.toLowerCase(),
          onPress: () => router.push(`/session/${row.id}`),
        };
      }),
    [pastSessions],
  );

  const sessionItems = useMemo(
    () => [...liveItems, ...pastItems].slice(0, MAX_SESSION_ITEMS),
    [liveItems, pastItems],
  );

  const connectionItems = useMemo<LauncherItem[]>(
    () => [
      {
        id: "conn:settings",
        title: "Settings",
        subtitle: host ?? undefined,
        keywords: "settings server host preferences",
        onPress: () => router.push("/settings"),
      },
      {
        id: "conn:repair",
        title: "Re-pair with a different server",
        keywords: "re-pair connect pairing url token qr scan new server",
        onPress: onRepair,
      },
      {
        id: "conn:forget",
        title: "Forget this server",
        keywords: "forget disconnect remove logout server",
        onPress: onForget,
      },
    ],
    [host, onRepair, onForget],
  );

  const appVersion = Constants.expoConfig?.version ?? "1.0.0";
  const aboutItems = useMemo<LauncherItem[]>(
    () => [
      {
        id: "about:version",
        title: "Forge",
        subtitle: `v${appVersion} · protocol 7`,
        keywords: "about version protocol app forge",
      },
      {
        id: "about:github",
        title: "Forge on GitHub",
        subtitle: FORGE_REPO_URL.replace(/^https:\/\//, ""),
        keywords: "github repo source docs code link",
        onPress: () => {
          Linking.openURL(FORGE_REPO_URL).catch(() => {});
        },
      },
    ],
    [appVersion],
  );

  const sections = useMemo<LauncherSection[]>(
    () => [
      { key: "session", label: "Session", items: sessionItems },
      { key: "connection", label: "Connection", items: connectionItems },
      { key: "about", label: "About", items: aboutItems },
    ],
    [sessionItems, connectionItems, aboutItems],
  );

  const normalizedQuery = query.trim().toLowerCase();
  const isSearching = normalizedQuery.length > 0;

  const visibleSections = useMemo(() => {
    if (!isSearching) return sections;
    return sections
      .map((section) => ({
        ...section,
        items: section.items.filter((item) => item.keywords.includes(normalizedQuery)),
      }))
      .filter((section) => section.items.length > 0);
  }, [sections, isSearching, normalizedQuery]);

  const recentItems = useMemo<LauncherItem[]>(() => {
    if (isSearching) return [];
    const combined = liveItems.length >= RECENT_COUNT ? liveItems : [...liveItems, ...pastItems];
    return combined.slice(0, RECENT_COUNT);
  }, [isSearching, liveItems, pastItems]);

  const isLoadingSessions =
    (sessionsQuery.isLoading || pastQuery.isLoading) && sessionItems.length === 0;

  let entranceIndex = 0;
  const nextIndex = () => entranceIndex++;

  return (
    <Screen contentContainerClassName="gap-16 pt-16">
      <EntranceView index={nextIndex()}>
        <View className="gap-4">
          <Text className="text-accent text-[16px] font-bold">⚒ More</Text>
          <Text className="text-dim text-[13px]">
            Search sessions, connection settings, and about Forge.
          </Text>
        </View>
      </EntranceView>

      <EntranceView index={nextIndex()}>
        <SearchInput
          value={query}
          onChangeText={setQuery}
          placeholder="Search sessions, settings, about…"
          autoCapitalize="none"
          autoCorrect={false}
        />
      </EntranceView>

      {!isSearching && recentItems.length > 0 ? (
        <View className="gap-6">
          <Text className="text-dim text-[11px] font-bold uppercase tracking-[0.5px]">
            Recent
          </Text>
          <View className={sectionWellClass()}>
            {recentItems.map((item) => (
              <EntranceView key={item.id} index={nextIndex()}>
                <ListRow
                  title={item.title}
                  subtitle={item.subtitle}
                  subtitleEllipsize="head"
                  right={item.right}
                  onPress={item.onPress}
                />
              </EntranceView>
            ))}
          </View>
        </View>
      ) : null}

      {isLoadingSessions ? <Loading label="Loading sessions…" /> : null}

      {visibleSections.map((section) => {
        const isCollapsed = !isSearching && collapsed[section.key];
        return (
          <View key={section.key} className="gap-6">
            <Pressable
              onPress={() => toggleSection(section.key)}
              hitSlop={8}
              className="flex-row items-center justify-between"
              style={{ minHeight: 32 }}
              accessibilityRole="button"
              accessibilityLabel={`${section.label}, ${section.items.length} items, ${isCollapsed ? "collapsed" : "expanded"}`}
            >
              <Text className="text-dim text-[11px] font-bold uppercase tracking-[0.5px]">
                {section.label} ({section.items.length})
              </Text>
              <Text className="text-dim text-[12px]">{isCollapsed ? "▸" : "▾"}</Text>
            </Pressable>
            {isCollapsed ? null : (
              <View className={sectionWellClass()}>
                {section.items.map((item) => (
                  <EntranceView key={item.id} index={nextIndex()}>
                    <ListRow
                      title={item.title}
                      subtitle={item.subtitle}
                      subtitleEllipsize={section.key === "session" ? "head" : "tail"}
                      right={item.right}
                      onPress={item.onPress}
                    />
                  </EntranceView>
                ))}
              </View>
            )}
          </View>
        );
      })}

      {isSearching && visibleSections.length === 0 ? (
        <EmptyState glyph="◌" title="No matches" />
      ) : null}

      {!isSearching && sessionItems.length >= MAX_SESSION_ITEMS ? (
        <Text className="text-dim text-[12px] text-center">
          Showing the first {MAX_SESSION_ITEMS} sessions — use History to search further back.
        </Text>
      ) : null}
    </Screen>
  );
}
