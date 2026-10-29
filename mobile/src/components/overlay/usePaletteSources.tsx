// The palette's local entries: live sessions, commands (actions, workbench, slash commands,
// skills, Go to, Anywhere) and settings destinations. Search-backed entries (message hits, files,
// other hosts) live in usePaletteSearch.
import { router } from "expo-router";
import {
  Archive,
  BellDot,
  Clock,
  Eye,
  Flame,
  History,
  LayoutPanelLeft,
  MoonStar,
  Plus,
  QrCode,
  Settings2,
  Sparkles,
  SunMedium,
  Wand2,
} from "lucide-react-native";
import React, { useMemo } from "react";
import { StyleSheet, Text, View } from "react-native";

import { ApiError } from "../../lib/api";
import { entitlementBadge, hostStateText } from "../../lib/anywhere/format";
import { useAnywhere, useAnywhereHosts } from "../../lib/anywhere/store";
import { runDesktopMenuAction, type DesktopMenuId } from "../../lib/desktopMenu";
import { haptics } from "../../lib/haptics";
import { mergeCommandSources } from "../../lib/commandSources";
import { BUILTIN_COMMANDS, useSkillCommands } from "../../lib/commands";
import { openSession } from "../../lib/nav";
import { SETTINGS_ENTRIES } from "../../lib/palette/settingsEntries";
import { configHref, settingsIndex } from "../../lib/settingsSearch";
import { recencyOf } from "../../lib/palette/recents";
import { formatShortcut, getResolvedShortcut, type AppShortcutAction, useShortcutLabel } from "../../lib/shortcuts";
import { useArchiveSession, useConfig, useSessions } from "../../lib/queries";
import { useTheme, useTokens } from "../../theme/ThemeProvider";
import { space, type StatusDotState } from "../../theme/tokens";
import { formatCwd, formatRelativeTime, type as typeScale } from "../../theme/typography";
import { HostDot } from "../anywhere/HostDot";
import { Badge } from "../ds/Badge";
import { IconButton } from "../ds/IconButton";
import { StatusDot } from "../ds/StatusDot";
import { useToast } from "../ds/ToastHost";
import { KeyHint } from "./PaletteRows";
import type { PaletteItem } from "./paletteTypes";

const ICON_SIZE = 18;
const ICON_STROKE = 1.75;

interface WorkbenchPaletteEntry {
  menu: DesktopMenuId;
  title: string;
  keywords: string;
  shortcut?: AppShortcutAction;
}

const WORKBENCH_PALETTE_ENTRIES: readonly WorkbenchPaletteEntry[] = [
  { menu: "view:split-pane", title: "Split pane", keywords: "split pane panes side by side multiple sessions column", shortcut: "workbench.split" },
  { menu: "view:pane-close", title: "Close pane", keywords: "close pane unsplit", shortcut: "workbench.closePane" },
  { menu: "view:pane-maximize", title: "Maximize pane", keywords: "maximize restore zoom pane focus", shortcut: "workbench.maximize" },
  { menu: "view:pane-next", title: "Focus next pane", keywords: "next pane focus cycle", shortcut: "workbench.paneNext" },
  { menu: "view:pane-prev", title: "Focus previous pane", keywords: "previous pane focus cycle", shortcut: "workbench.panePrev" },
  { menu: "view:layout-focus", title: "Layout: Focus", keywords: "layout preset focus single one pane" },
  { menu: "view:layout-side-by-side", title: "Layout: Side by side", keywords: "layout preset side by side two panes split" },
  { menu: "view:layout-grid", title: "Layout: Grid 2 x 2", keywords: "layout preset grid four panes" },
  { menu: "view:layout-review", title: "Layout: Review", keywords: "layout preset review git diff tasks" },
  { menu: "view:layout-reset", title: "Reset layout", keywords: "reset layout default panes docks sizes" },
  { menu: "view:sidebar", title: "Toggle sidebar", keywords: "sidebar rail collapse expand", shortcut: "shell.sidebar" },
  { menu: "view:tasks", title: "Toggle Tasks panel", keywords: "tasks todo list dock panel right", shortcut: "workbench.tasks" },
  { menu: "view:agents", title: "Toggle Agents panel", keywords: "agents subagents dock panel right", shortcut: "workbench.agents" },
  { menu: "view:plan", title: "Toggle Plan panel", keywords: "plan steps dock panel right", shortcut: "workbench.plan" },
  { menu: "view:files", title: "Toggle Files panel", keywords: "files workspace browser dock panel right", shortcut: "workbench.files" },
  { menu: "view:git-review", title: "Toggle Git review", keywords: "git review diff changes dock panel right", shortcut: "workbench.gitReview" },
  { menu: "view:terminal", title: "Toggle Terminal", keywords: "terminal shell dock bottom", shortcut: "workbench.terminal" },
  { menu: "view:browser-preview", title: "Toggle Browser preview", keywords: "browser preview web dock panel right" },
  { menu: "view:gallery", title: "Toggle Gallery", keywords: "gallery images screenshots pictures evidence dock panel right" },
  { menu: "view:usage", title: "Toggle usage panel", keywords: "usage dock panel limits quota subscription spend right sidebar", shortcut: "shell.usage" },
];

export interface PaletteSourceOptions {
  close: () => void;
  isCompact: boolean;
  activeSessionId: string | null;
  recents: readonly string[];
  openWhatsNew: () => void;
  openSchedules: () => void;
  peekDecision: (sessionId: string) => void;
  runSlashCommand: (command: string) => void;
}

export function liveSessionState(row: { waiting: boolean; busy: boolean }): StatusDotState {
  return row.waiting ? "waiting" : row.busy ? "busy" : "idle";
}

export function usePaletteSources(options: PaletteSourceOptions) {
  const { close, isCompact, activeSessionId, recents, openWhatsNew, openSchedules, peekDecision, runSlashCommand } = options;
  const tokens = useTokens();
  const { scheme, setScheme } = useTheme();
  const toast = useToast();
  const { data: sessions } = useSessions();
  const archiveSession = useArchiveSession();
  const skillCommands = useSkillCommands();
  const { signedIn: anywhereSignedIn, account: anywhereAccount } = useAnywhere();
  const { hosts: anywhereHosts } = useAnywhereHosts();
  const newSessionHint = useShortcutLabel("session.new");
  const fleetHint = useShortcutLabel("nav.fleet");
  const inboxHint = useShortcutLabel("nav.inbox");
  const historyHint = useShortcutLabel("nav.history");
  const settingsHint = useShortcutLabel("nav.settings");

  const sessionItems = useMemo<PaletteItem[]>(
    () =>
      (sessions ?? []).map((s) => {
        const title = s.title || `session ${s.id.slice(0, 8)}`;
        const status = s.waiting ? "needs you" : s.busy ? "forging" : formatRelativeTime(s.last_activity * 1000);
        return {
          id: `session:${s.id}`,
          section: "sessions",
          sessionId: s.id,
          title,
          subtitle: `${formatCwd(s.cwd)} · ${status}`,
          keywords: `${s.cwd} ${s.model}`,
          priority: s.waiting,
          recency: s.last_activity,
          leading: <StatusDot state={liveSessionState(s)} />,
          trailing: s.waiting ? (
            <View style={styles.trailingRow}>
              <Badge label="NEEDS YOU" tone="danger" />
              <IconButton
                icon={<Eye size={16} strokeWidth={ICON_STROKE} color={tokens.ink3} />}
                onPress={() => {
                  close();
                  peekDecision(s.id);
                }}
                accessibilityLabel={`Review decision for ${title}`}
              />
            </View>
          ) : undefined,
          trailingInteractive: s.waiting,
          onSelect: () => {
            close();
            openSession(s.id);
          },
        };
      }),
    [sessions, tokens, close, peekDecision],
  );

  const commandItems = useMemo<PaletteItem[]>(() => {
    const icon = (node: React.ReactNode) => node;
    const items: PaletteItem[] = [
      {
        id: "action:new-session",
        section: "commands",
        title: "Forge a session",
        keywords: "new session create start forge task",
        leading: <Plus size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />,
        trailing: !isCompact ? <KeyHint label={newSessionHint} /> : undefined,
        onSelect: () => {
          close();
          router.push("/new-session");
        },
      },
    ];

    if (activeSessionId) {
      items.push({
        id: "action:archive-current",
        section: "commands",
        title: "Archive current session",
        keywords: "archive stop hide current",
        leading: <Archive size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />,
        onSelect: () => {
          const id = activeSessionId;
          close();
          archiveSession.mutate(id, {
            onSuccess: () => {
              haptics.allow();
              toast.show("session archived");
            },
            onError: (err) => {
              haptics.mergeConflict();
              toast.show(err instanceof ApiError ? err.message : "archive failed", { tone: "danger" });
            },
          });
        },
      });
    }

    // The workbench (panes, docks, layouts) lives in the wide desktop shell only; Linux and
    // Windows have no menu bar, so this is their menu-equivalent route to every entry.
    if (!isCompact) {
      for (const entry of WORKBENCH_PALETTE_ENTRIES) {
        items.push({
          id: `action:${entry.menu}`,
          section: "commands",
          title: entry.title,
          keywords: entry.keywords,
          leading: <LayoutPanelLeft size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />,
          trailing: entry.shortcut ? <KeyHint label={formatShortcut(getResolvedShortcut(entry.shortcut))} /> : undefined,
          onSelect: () => {
            close();
            runDesktopMenuAction(entry.menu);
          },
        });
      }
    }

    items.push({
      id: "action:theme-toggle",
      section: "commands",
      title: scheme === "dark" ? "Switch to light theme" : "Switch to dark theme",
      keywords: "theme dark light appearance",
      leading: icon(
        scheme === "dark" ? (
          <SunMedium size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />
        ) : (
          <MoonStar size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />
        ),
      ),
      onSelect: () => {
        close();
        setScheme(scheme === "dark" ? "light" : "dark");
      },
    });

    items.push({
      id: "action:whats-new",
      section: "commands",
      title: "What's New",
      keywords: "whats new changelog release notes updates",
      leading: <Sparkles size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />,
      onSelect: () => {
        close();
        openWhatsNew();
      },
    });

    items.push({
      id: "action:schedules",
      section: "commands",
      title: "Schedules",
      keywords: "schedules recurring cron scheduled runs",
      leading: <Clock size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />,
      onSelect: () => {
        close();
        openSchedules();
      },
    });

    const skillNames = new Set(skillCommands.map((skill) => skill.name));
    for (const cmd of mergeCommandSources(BUILTIN_COMMANDS, skillCommands)) {
      const isSkill = skillNames.has(cmd.name);
      items.push({
        id: `action:cmd:${cmd.name}`,
        section: "commands",
        title: cmd.name,
        subtitle: cmd.description || (activeSessionId ? "send to the current session" : "open a session first"),
        keywords: isSkill ? `skill ${cmd.name}` : cmd.name,
        disabled: !activeSessionId,
        leading: <Wand2 size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />,
        trailing: isSkill ? <Badge label="SKILL" tone="neutral" /> : undefined,
        onSelect: () => runSlashCommand(cmd.name),
      });
    }

    const goTo: { id: string; title: string; href: "/" | "/inbox" | "/history"; hint: string; keywords: string; Icon: typeof Flame }[] = [
      { id: "nav:fleet", title: "Go to Fleet", href: "/", hint: fleetHint, keywords: "fleet home sessions live", Icon: Flame },
      { id: "nav:inbox", title: "Go to Inbox", href: "/inbox", hint: inboxHint, keywords: "inbox waiting decisions needs you", Icon: BellDot },
      { id: "nav:history", title: "Go to History", href: "/history", hint: historyHint, keywords: "history past archived", Icon: History },
    ];
    for (const entry of goTo) {
      items.push({
        id: entry.id,
        section: "commands",
        title: entry.title,
        keywords: entry.keywords,
        leading: <entry.Icon size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />,
        trailing: !isCompact ? <KeyHint label={entry.hint} /> : undefined,
        onSelect: () => {
          close();
          router.push(entry.href);
        },
      });
    }

    if (anywhereSignedIn) {
      for (const host of anywhereHosts) {
        items.push({
          id: `anywhere:host:${host.id}`,
          section: "commands",
          title: `Go to host: ${host.name}`,
          keywords: `anywhere host ${host.name}`,
          trailing: (
            <View style={styles.trailingRow}>
              <HostDot state={host.state} size={5} />
              <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>{hostStateText(host)}</Text>
            </View>
          ),
          onSelect: () => {
            close();
            router.push(`/anywhere/host/${host.id}`);
          },
        });
      }
      items.push({
        id: "anywhere:pair",
        section: "commands",
        title: "Pair a new device…",
        keywords: "anywhere pair device qr enroll",
        leading: <QrCode size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />,
        onSelect: () => {
          close();
          router.push("/anywhere/pair");
        },
      });
      items.push({
        id: "anywhere:settings",
        section: "commands",
        title: "Anywhere settings",
        keywords: "anywhere settings entitlement plan billing devices hosts storage",
        leading: <Settings2 size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />,
        trailing: anywhereAccount ? (
          <Text style={[typeScale.monoMeta, { color: tokens.ink3 }]}>{entitlementBadge(anywhereAccount).label.toLowerCase()}</Text>
        ) : undefined,
        onSelect: () => {
          close();
          router.push("/anywhere");
        },
      });
    }

    return items.map((item) => ({ ...item, recency: recencyOf(recents, item.id) }));
  }, [
    tokens, activeSessionId, scheme, setScheme, close, archiveSession, toast, runSlashCommand, skillCommands, isCompact,
    newSessionHint, fleetHint, inboxHint, historyHint, anywhereSignedIn, anywhereHosts, anywhereAccount, recents,
    openWhatsNew, openSchedules,
  ]);

  const settingsItems = useMemo<PaletteItem[]>(
    () =>
      SETTINGS_ENTRIES.map((entry) => ({
        id: `settings:${entry.href}`,
        section: "settings",
        title: entry.href === "/settings" ? "General settings" : entry.title,
        subtitle: entry.subtitle,
        keywords: entry.keywords,
        recency: recencyOf(recents, `settings:${entry.href}`),
        leading: <Settings2 size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />,
        trailing: entry.href === "/settings" && !isCompact ? <KeyHint label={settingsHint} /> : undefined,
        onSelect: () => {
          close();
          router.push(entry.href);
        },
      })),
    [tokens, close, isCompact, settingsHint, recents],
  );

  const configQuery = useConfig();
  const settingFieldItems = useMemo<PaletteItem[]>(
    () =>
      settingsIndex(configQuery.data?.fields ?? [])
        .filter((hit) => hit.kind === "field")
        .map((hit) => ({
          id: `settings:${hit.id}`,
          section: "settings" as const,
          title: hit.title,
          subtitle: hit.subtitle,
          keywords: hit.keywords,
          recency: recencyOf(recents, `settings:${hit.id}`),
          leading: <Settings2 size={ICON_SIZE} strokeWidth={ICON_STROKE} color={tokens.ink2} />,
          onSelect: () => {
            close();
            router.push(configHref(hit.focusKey));
          },
        })),
    [configQuery.data?.fields, tokens, close, recents],
  );

  const allSettingsItems = useMemo(() => [...settingsItems, ...settingFieldItems], [settingsItems, settingFieldItems]);

  return { sessions, sessionItems, commandItems, settingsItems: allSettingsItems };
}

const styles = StyleSheet.create({
  trailingRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
});
