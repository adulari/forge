// Machined desktop shell — "What's New" (D What's New, docs/design/machined INVENTORY.md
// L384-390). Backed by `GET /api/changelog`, which serves the daemon's compiled-in CHANGELOG:
// one block per release, entries grouped under their `### Added` / `### Changed` heading.
// The newest release renders expanded with a NEW badge; older ones collapse to a one-glance
// summary and open on tap. Reached from the command palette's Actions group.
import { Sparkles } from "lucide-react-native";
import React, { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { type ChangelogEntry, type ChangelogRelease } from "../../lib/api";
import { useChangelog } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { tabularNums, type as typeScale } from "../../theme/typography";
import { EmptyState } from "../ds/EmptyState";
import { Sheet } from "../ds/Sheet";

const RELEASE_LIMIT = 12;

/** `2026-07-02` → `Jul 2` (`Jul 2 2025` across a year boundary). Anything unparseable is
 * shown verbatim rather than guessed at. */
function formatReleaseDate(date: string | null): string | null {
  if (!date) return null;
  const parsed = new Date(date);
  if (Number.isNaN(parsed.getTime())) return date;
  const sameYear = parsed.getFullYear() === new Date().getFullYear();
  return parsed.toLocaleDateString("en", {
    month: "short",
    day: "numeric",
    ...(sameYear ? {} : { year: "numeric" }),
  });
}

function groupBySection(entries: ChangelogEntry[]): { section: string; texts: string[] }[] {
  const groups: { section: string; texts: string[] }[] = [];
  for (const entry of entries) {
    const last = groups[groups.length - 1];
    if (last && last.section === entry.section) last.texts.push(entry.text);
    else groups.push({ section: entry.section, texts: [entry.text] });
  }
  return groups;
}

function ReleaseBlock({ release, latest }: { release: ChangelogRelease; latest: boolean }) {
  const tokens = useTokens();
  const [expanded, setExpanded] = useState(latest);
  const date = formatReleaseDate(release.date);

  return (
    <View style={styles.release}>
      <Pressable
        onPress={() => setExpanded((open) => !open)}
        accessibilityRole="button"
        accessibilityLabel={`${release.version}${date ? `, ${date}` : ""}`}
        accessibilityState={{ expanded }}
        style={styles.releaseHead}
      >
        <Text style={[typeScale.bodyBold, { color: expanded ? tokens.ink : tokens.ink2 }]}>{release.version}</Text>
        {latest ? (
          <Text style={[typeScale.monoMeta, { color: tokens.accent }]}>NEW</Text>
        ) : date ? (
          <Text style={[typeScale.monoMeta, tabularNums, { color: tokens.ink3 }]}>{date}</Text>
        ) : null}
      </Pressable>

      {expanded ? (
        groupBySection(release.entries).map((group) => (
          <View key={group.section} style={styles.group}>
            {group.section ? (
              <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]}>{group.section.toUpperCase()}</Text>
            ) : null}
            {group.texts.map((text, index) => (
              <Text key={index} style={[typeScale.sub, { color: tokens.ink2 }]}>
                {text}
              </Text>
            ))}
          </View>
        ))
      ) : (
        <Text style={[typeScale.sub, { color: tokens.ink3 }]} numberOfLines={2}>
          {release.entries.map((entry) => entry.text).join(" · ")}
        </Text>
      )}
    </View>
  );
}

export interface WhatsNewPanelProps {
  visible: boolean;
  onClose: () => void;
}

export function WhatsNewPanel({ visible, onClose }: WhatsNewPanelProps) {
  const tokens = useTokens();
  const query = useChangelog(RELEASE_LIMIT);
  const releases = query.data ?? [];

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="What's New">
      <View style={styles.content}>
        <Text style={[typeScale.headingBold, { color: tokens.ink }]}>What&apos;s New</Text>
        {query.isLoading && releases.length === 0 ? (
          <Text style={[typeScale.sub, styles.loading, { color: tokens.ink3 }]}>Loading release notes…</Text>
        ) : releases.length === 0 ? (
          <EmptyState
            icon={Sparkles}
            message="No release notes from this daemon — it was built without a changelog, or predates the feed."
          />
        ) : (
          releases.map((release, index) => (
            <ReleaseBlock key={release.version} release={release} latest={index === 0} />
          ))
        )}
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space16, paddingBottom: space.space32, gap: space.space12 },
  loading: { paddingVertical: space.space16 },
  release: { gap: space.space4 },
  releaseHead: { flexDirection: "row", alignItems: "baseline", gap: space.space8, minHeight: 22 },
  group: { gap: 2, paddingTop: space.space4 },
});
