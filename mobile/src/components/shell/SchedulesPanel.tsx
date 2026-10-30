// Machined desktop shell - "Schedules" (D Schedules, docs/design/machined INVENTORY.md L395-403).
// Backed by the daemon's real schedule store (`/api/schedules`, crates/forge-cli/src/
// serve_schedules.rs): a cadence builder, filters and search, Run now, per-row last results that
// link to the session they created, and a results inbox with read / unread. Reached from the
// sidebar footer and the command palette's Actions group.
import { Clock } from "lucide-react-native";
import React, { useState } from "react";
import { Pressable, StyleSheet, Text, useWindowDimensions, View } from "react-native";

import type { ScheduleRow } from "../../lib/api";
import { useScheduleRuns, useSchedules } from "../../lib/queries";
import { usePrWatches } from "../../lib/queriesPr";
import { activeWorkspaceProjects, useShellPrefs } from "../../lib/shellPrefs";
import { projectName } from "../../lib/sidebarModel";
import { useTokens } from "../../theme/ThemeProvider";
import { hexToRgba, radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { EmptyState } from "../ds/EmptyState";
import { SearchField } from "../ds/SearchField";
import { Segmented } from "../ds/Segmented";
import { Sheet } from "../ds/Sheet";
import { PrWatchList } from "./schedules/PrWatchList";
import { ScheduleForm } from "./schedules/ScheduleForm";
import { ScheduleResults, unreadCount } from "./schedules/ScheduleResults";
import { ScheduleRowView } from "./schedules/ScheduleRowView";
import {
  filterSchedules,
  SCHEDULE_TEMPLATES,
  type ScheduleFilter,
  type ScheduleTemplate,
} from "./schedules/scheduleModel";

type Tab = "schedules" | "results";

export interface SchedulesPanelProps {
  visible: boolean;
  onClose: () => void;
}

function TemplateCards({ onPick }: { onPick: (template: ScheduleTemplate) => void }) {
  const tokens = useTokens();
  return (
    <View style={styles.templates}>
      {SCHEDULE_TEMPLATES.map((template) => (
        <Pressable
          key={template.id}
          onPress={() => onPick(template)}
          accessibilityRole="button"
          accessibilityLabel={`Use the ${template.title} template`}
          style={({ pressed, hovered }: { pressed: boolean; hovered?: boolean }) => [
            styles.template,
            { borderColor: tokens.border },
            (pressed || hovered) && { backgroundColor: hexToRgba(tokens.accent, 0.1) },
          ]}
        >
          <Text style={[typeScale.sub, { color: tokens.ink }]}>{template.title}</Text>
          <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>{template.description}</Text>
        </Pressable>
      ))}
    </View>
  );
}

export function SchedulesPanel({ visible, onClose }: SchedulesPanelProps) {
  const tokens = useTokens();
  const wide = useWindowDimensions().width >= 640;
  const query = useSchedules();
  const runsQuery = useScheduleRuns(undefined, visible);
  const [tab, setTab] = useState<Tab>("schedules");
  const [filter, setFilter] = useState<ScheduleFilter>("all");
  const [search, setSearch] = useState("");
  const [form, setForm] = useState<{ editing: ScheduleRow | null; template: ScheduleTemplate | null } | null>(null);
  const workspaceProjects = activeWorkspaceProjects(useShellPrefs());
  const all = (query.data ?? []).filter((row) => !workspaceProjects || workspaceProjects.has(projectName(row.cwd)));
  const rows = filterSchedules(all, filter, search);
  const unread = unreadCount(runsQuery.data);
  const watches = (usePrWatches().data?.watches ?? []).filter((w) => w.mode !== "off");

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Schedules">
      <View style={styles.content}>
        <View style={styles.head}>
          <Text style={[typeScale.headingBold, styles.headTitle, { color: tokens.ink }]}>Schedules</Text>
          <Pressable
            onPress={() => setForm(form ? null : { editing: null, template: null })}
            accessibilityRole="button"
            accessibilityLabel="New schedule"
            accessibilityState={{ expanded: form !== null && form.editing === null }}
            style={({ pressed }) => [
              styles.newButton,
              { borderColor: tokens.border },
              pressed && { backgroundColor: hexToRgba(tokens.accent, 0.12) },
            ]}
          >
            <Text style={[typeScale.monoMeta, { color: tokens.ink2 }]}>+ New schedule</Text>
          </Pressable>
        </View>

        <Segmented<Tab>
          options={[
            { value: "schedules", label: "Schedules" },
            { value: "results", label: "Results", badge: unread || undefined },
          ]}
          value={tab}
          onChange={setTab}
        />

        {form ? (
          <ScheduleForm
            key={form.editing?.id ?? form.template?.id ?? "new"}
            editing={form.editing}
            template={form.template}
            onDone={() => setForm(null)}
          />
        ) : null}

        {tab === "results" ? (
          <ScheduleResults schedules={all} onNavigate={onClose} />
        ) : (
          <>
            {all.length > 0 ? (
              <View style={wide ? styles.filtersWide : styles.filters}>
                <SearchField
                  value={search}
                  onChangeText={setSearch}
                  placeholder="Search schedules"
                  showCancel={false}
                  containerStyle={wide ? styles.search : undefined}
                />
                <Segmented<ScheduleFilter>
                  options={[
                    { value: "all", label: "All" },
                    { value: "active", label: "Active" },
                    { value: "paused", label: "Paused" },
                  ]}
                  value={filter}
                  onChange={setFilter}
                />
              </View>
            ) : null}

            <PrWatchList watches={watches} onNavigate={onClose} />

            {query.isLoading && all.length === 0 ? (
              <Text style={[typeScale.sub, styles.loading, { color: tokens.ink3 }]}>Loading schedules...</Text>
            ) : all.length === 0 ? (
              <View style={styles.empty}>
                <EmptyState icon={Clock} message="No schedules yet. Start from a template or create your own." />
                {form ? null : <TemplateCards onPick={(template) => setForm({ editing: null, template })} />}
              </View>
            ) : rows.length === 0 ? (
              <Text style={[typeScale.sub, styles.loading, { color: tokens.ink3 }]}>No schedules match.</Text>
            ) : (
              <View style={styles.list}>
                {rows.map((row) => (
                  <ScheduleRowView
                    key={row.id}
                    row={row}
                    onEdit={(editing) => setForm({ editing, template: null })}
                    onNavigate={onClose}
                  />
                ))}
              </View>
            )}
          </>
        )}

        <Text style={[typeScale.monoMeta, styles.footer, { color: tokens.ink4, borderTopColor: tokens.border }]}>
          Schedules run via your OS scheduler, even when Forge is closed. Each run is an ordinary session in Fleet.
        </Text>
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.space16, paddingBottom: space.space32, gap: space.space12 },
  head: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  headTitle: { flex: 1 },
  newButton: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4, paddingHorizontal: 8, paddingVertical: 4 },
  filters: { gap: space.space8 },
  filtersWide: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  search: { flex: 1, minWidth: 0 },
  list: { paddingVertical: space.space4 },
  loading: { paddingVertical: space.space16 },
  empty: { gap: space.space12 },
  templates: { gap: space.space8 },
  template: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius8, padding: space.space12, gap: 2 },
  footer: { borderTopWidth: StyleSheet.hairlineWidth, paddingTop: space.space8 },
});
