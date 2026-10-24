// Webview half of the native desktop surfaces (docs/design/machined INVENTORY.md § 08 Native —
// "D Menus" and "D Tray Notif About"). The Tauri shell (`src-tauri/src/menu.rs`, `tray.rs`)
// holds no daemon state: activating any menu or tray item just emits `forge://menu` with a
// stable id, and this module turns that id into the real in-app action.
//
// Three dispatch tiers, in priority order:
//   1. A handler registered by a live component (`useDesktopMenuAction`) — used for actions that
//      need state only that component has (the sidebar toggle, the open session's socket).
//   2. A context-free default here (router navigation, external links).
//   3. `unwired()` — the action belongs to a surface that does not exist yet. It logs the exact
//      event id once in dev so the builder who lands that surface can grep for it.
//
// Everything is inert off Tauri: `useDesktopMenu` returns before importing any `@tauri-apps/*`
// module, so web/iOS/Android bundles never pull the desktop APIs in.
import AsyncStorage from "@react-native-async-storage/async-storage";
import { router } from "expo-router";
import { useEffect, useRef } from "react";

import { isAppLockEngaged } from "../components/AppLock";
import { answer as answerPermission } from "./api";
import { useAuth } from "./auth";
import { decisionCopy, isContentLocked, redactTrayTitle } from "./desktopNotifyCore";
import { isTauri } from "./platform";
import { useDiagnostics, useSessions } from "./queries";
import {
  answerTrayDecision,
  buildTrayDecision,
  newestWaiting,
  type TrayDecisionSummary,
  type TrayDecisionTarget,
} from "./trayDecisionCore";
import { useSessionSocket } from "./ws";
import { checkDesktopUpdate } from "./updater";
import { PROTOCOL_VERSION } from "./remoteProtocol";
import { useTheme } from "../theme/ThemeProvider";
import { formatRelativeTime } from "../theme/typography";

/** Mirrors `menu::MENU_EVENT` in src-tauri/src/menu.rs. */
const MENU_EVENT = "forge://menu";

/** Mirrors `tray::TRAY_SESSION_PREFIX`. */
const TRAY_SESSION_PREFIX = "tray:session:";
const APPEARANCE_PREFIX = "view:appearance:";

/** ThemeProvider's own `STORAGE_KEY` (theme/ThemeProvider.tsx) — not exported there, so it is
 * duplicated rather than imported. Keep the two in sync. */
const THEME_STORAGE_KEY = "forge.theme";

const REPO_URL = "https://github.com/Adulari/forge";

/** Matches `tray::MAX_SESSIONS` — the shell truncates anyway, so don't ship rows it will drop. */
const TRAY_SESSION_LIMIT = 6;

/** Every id `src-tauri/src/menu.rs` and `tray.rs` can emit, minus the dynamic
 * `tray:session:<id>` rows. Kept as a union so a typo in a `useDesktopMenuAction` call is a
 * compile error rather than a silently dead menu item. */
export type DesktopMenuId =
  | "app:about"
  | "app:check-updates"
  | "app:settings"
  | "session:new"
  | "session:quick-composer"
  | "session:search"
  | "session:approve"
  | "session:interrupt"
  | "session:fork"
  | "session:checkpoint"
  | "session:handoff"
  | "session:share-replay"
  | "session:archive"
  | "view:sidebar"
  | "view:split-pane"
  | "view:terminal"
  | "view:usage"
  | "view:notes"
  | "view:git-review"
  | "view:browser-preview"
  | "view:appearance:light"
  | "view:appearance:dark"
  | "view:appearance:system"
  | "help:docs"
  | "help:issue"
  | "help:acknowledgements"
  | "tray:open"
  | "tray:quick-composer"
  | "tray:decision:allow"
  | "tray:decision:deny"
  | "tray:decision:review";

export type DesktopMenuHandler = () => void;

// A stack rather than a single slot: two components can legitimately claim the same id (the
// fleet list and an open session both want `session:archive`), and the most recently mounted
// one is the one the user is looking at.
const registry = new Map<string, DesktopMenuHandler[]>();

/** Imperative form of `useDesktopMenuAction`. Returns an unregister function. */
export function registerDesktopMenuAction(
  id: DesktopMenuId,
  handler: DesktopMenuHandler,
): () => void {
  const stack = registry.get(id) ?? [];
  stack.push(handler);
  registry.set(id, stack);
  return () => {
    const current = registry.get(id);
    if (!current) return;
    const index = current.indexOf(handler);
    if (index >= 0) current.splice(index, 1);
  };
}

/**
 * Claim a menu/tray item for as long as the calling component is mounted. Safe to call on every
 * platform — off Tauri nothing ever dispatches, so the registration is simply never used.
 */
export function useDesktopMenuAction(id: DesktopMenuId, handler: DesktopMenuHandler): void {
  // The handler identity usually changes every render; registering a stable trampoline keeps
  // the effect from tearing the registration down and rebuilding it on each one.
  const latest = useRef(handler);
  useEffect(() => {
    latest.current = handler;
  });
  useEffect(() => registerDesktopMenuAction(id, () => latest.current()), [id]);
}

const warned = new Set<string>();

function unwired(id: string, owner: string): void {
  if (warned.has(id) || !__DEV__) return;
  warned.add(id);
  console.warn(
    `[desktopMenu] "${id}" has no handler yet — ${owner} should call useDesktopMenuAction("${id}", …).`,
  );
}

async function openExternal(url: string): Promise<void> {
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  await openUrl(url);
}

/** Fallback for when `useDesktopNativeSync` (which registers the live `setScheme`) is not
 * mounted: ThemeProvider sits below this module's own mount point, so all we can do is write
 * the preference it reads at startup — the choice then applies on next launch. */
async function persistAppearance(preference: "light" | "dark" | "system"): Promise<void> {
  await AsyncStorage.setItem(THEME_STORAGE_KEY, preference);
}

function runDefault(id: string): void {
  switch (id) {
    case "app:settings":
      router.push("/settings");
      return;
    // Diagnostics & updates owns the desktop updater UI. It only checks on mount while the
    // updater is idle, so a second "Check for Updates…" has to ask for the check itself.
    case "app:check-updates":
      router.push("/diagnostics");
      void checkDesktopUpdate().catch(() => undefined);
      return;
    case "session:new":
      router.push("/new-session");
      return;
    // The waiting decisions the design's "Approve" item acts on are exactly Inbox's contents;
    // an open session claims this id itself and answers in place.
    case "session:approve":
      router.push("/inbox");
      return;
    case "session:handoff":
      router.push("/anywhere/handoff");
      return;
    case "session:fork":
      router.push("/session-tree");
      return;
    case "tray:open":
      router.push("/");
      return;
    case "help:docs":
      void openExternal(`${REPO_URL}#readme`);
      return;
    case "help:issue":
      void openExternal(`${REPO_URL}/issues/new`);
      return;
    case "help:acknowledgements":
      void openExternal(`${REPO_URL}/blob/main/LICENSE`);
      return;
    default:
      break;
  }

  if (id.startsWith(TRAY_SESSION_PREFIX)) {
    router.push(`/session/${id.slice(TRAY_SESSION_PREFIX.length)}`);
    return;
  }

  if (id.startsWith(APPEARANCE_PREFIX)) {
    const preference = id.slice(APPEARANCE_PREFIX.length);
    if (preference === "light" || preference === "dark" || preference === "system") {
      void persistAppearance(preference);
    }
    return;
  }

  switch (id) {
    // Shell chrome — claimed by the hooks in `shortcuts/useShellHotkeys.ts`, which run inside
    // RootNavigator and hold the actual toggles. Reaching here means the shell is unmounted
    // (unpaired, or a compact window), where the toggle has nothing to act on.
    case "session:quick-composer":
    case "tray:quick-composer":
    case "session:search":
    case "view:sidebar":
    case "view:usage":
      return;
    // Session-scoped — the open session registers these; nothing to do with no session open.
    case "session:interrupt":
    case "session:checkpoint":
    case "session:archive":
      return;
    case "session:share-replay":
      unwired(id, "the replay-share builder (ShareSheet)");
      return;
    // Docked panels being built in parallel (DockHost's registry is `DockKind`-typed and
    // currently only has "usage"). Each becomes one `useDesktopMenuAction` call in the
    // component that owns the dock.
    case "view:split-pane":
    case "view:terminal":
    case "view:notes":
    case "view:git-review":
    case "view:browser-preview":
      unwired(id, "the split-pane/dock builder (components/shell/DockHost.tsx)");
      return;
    default:
      unwired(id, "nobody");
  }
}

/** Run a menu action as if its menu item had been chosen — for in-app surfaces (the command
 * palette) that offer the same action on platforms without a native menu bar. */
export function runDesktopMenuAction(id: DesktopMenuId): void {
  dispatch(id);
}

function dispatch(id: string): void {
  const stack = registry.get(id);
  const handler = stack?.[stack.length - 1];
  if (handler) {
    handler();
    return;
  }
  runDefault(id);
}

// ---------------------------------------------------------------------------
// Tray + About: webview → shell pushes
// ---------------------------------------------------------------------------

export interface TraySessionSummary {
  id: string;
  title: string;
  state: "waiting" | "busy" | "idle";
  /** Trailing mono caption in the design (`4m`, `atlas`). */
  meta?: string;
}

export interface TraySummary {
  busy: number;
  waiting: number;
  costUsd?: number;
  /** The newest waiting decision. `title` is pre-redacted (`decisionCopy`); `promptSeq` is set
   * only when the tray may offer Allow/Deny — see `trayDecisionCore.ts`. */
  decision?: TrayDecisionSummary;
  sessions: TraySessionSummary[];
}

/**
 * Push a compact fleet summary into the menu-bar extra. The shell never queries the daemon, so
 * this is the tray's only data source; call it whenever fleet state changes — or just mount
 * `useDesktopNativeSync`, which does exactly that.
 *
 * PRIVACY: whatever is passed here is rendered in the OS menu bar, outside the app's lock. Titles
 * for Anywhere-routed sessions must already be generic — use `redactTrayTitle` from
 * `desktopNotifyCore.ts` rather than passing raw titles through.
 */
export async function pushTraySummary(summary: TraySummary): Promise<void> {
  if (!isTauri) return;
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke("set_tray_summary", { summary });
  } catch {
    // Best-effort: a tray that failed to build (no StatusNotifier host on Linux) must not break
    // the caller's render path.
  }
}

export interface AboutInfo {
  appVersion?: string;
  build?: string;
  protocol?: string;
  daemonVersion?: string;
  host?: string;
}

/** Fills in the About panel's two mono lines. `useDesktopMenu` pushes the app-level facts at
 * mount; the daemon version/host need a paired server, so whoever holds that state pushes them. */
export async function pushAboutInfo(info: AboutInfo): Promise<void> {
  if (!isTauri) return;
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke("set_about_info", { info });
  } catch {
    // Best-effort — About falls back to the shell's own version string.
  }
}

/** Opens the native About panel (also reachable from the Forge menu). */
export async function openAboutWindow(): Promise<void> {
  if (!isTauri) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("open_about_window");
}

// ---------------------------------------------------------------------------
// Mount
// ---------------------------------------------------------------------------

/**
 * The half of the native wiring that needs app state: the tray's live fleet summary, the
 * Appearance submenu's live theme switch, and the About panel's daemon/host line.
 *
 * MOUNT: exactly once, from any component INSIDE the provider tree (ThemeProvider + AuthProvider
 * + the query client) — `components/fleet/FleetWatcher.tsx` is the natural home, since it
 * already watches the same fleet state for toasts. `useDesktopMenu` cannot do this itself: it is
 * mounted from `useGlobalShortcuts` in RootLayout's own body, which runs above every provider.
 * No-op on web/iOS/Android.
 */
export function useDesktopNativeSync(): void {
  const { servers, activeServerId } = useAuth();
  const { setScheme } = useTheme();
  const { data: sessions } = useSessions();
  const { data: diagnostics } = useDiagnostics();
  const lastPushed = useRef<string | null>(null);

  useDesktopMenuAction("view:appearance:light", () => setScheme("light"));
  useDesktopMenuAction("view:appearance:dark", () => setScheme("dark"));
  useDesktopMenuAction("view:appearance:system", () => setScheme("system"));
  const activeServer = servers.find((server) => server.id === activeServerId);
  const host = activeServer?.name;
  // The About panel's "daemon x.y.z · host" line; without the version it showed only the host.
  const daemonVersion = diagnostics?.host.version;
  // Anywhere-routed sessions are end-to-end encrypted; their titles must not reach the menu bar.
  const locked = isContentLocked(activeServer?.transport);

  useEffect(() => {
    if (!isTauri) return;
    void pushAboutInfo({ host, daemonVersion });
  }, [host, daemonVersion]);

  // The tray's decision rows need what the fleet list does not carry — whether the newest
  // waiting session is asking a permission (yes/no) or a question, and its `prompt_seq` — so
  // that one session is watched over its socket, the same short attach DecisionCard uses.
  const newest = newestWaiting(sessions ?? []);
  const { snapshot: decisionSnapshot } = useSessionSocket(
    isTauri && newest ? (activeServer?.baseUrl ?? null) : null,
    isTauri ? (newest?.id ?? null) : null,
  );
  const decision = newest
    ? buildTrayDecision({
        title: decisionCopy({ sessionTitle: newest.title, locked }).title,
        serverId: activeServerId,
        sessionId: newest.id,
        snapshot: decisionSnapshot,
        contentLocked: locked,
      })
    : undefined;

  const live = useRef({ servers, activeServerId, decisionSnapshot });
  useEffect(() => {
    live.current = { servers, activeServerId, decisionSnapshot };
  }, [servers, activeServerId, decisionSnapshot]);
  useEffect(() => {
    if (!isTauri) return;
    decisionHandler = (id, target) => {
      const { servers: stored, activeServerId: activeId, decisionSnapshot: snap } = live.current;
      const openTarget = () => {
        void openMainWindow();
        router.push(target && target.serverId === activeId ? `/session/${target.sessionId}` : "/inbox");
      };
      if (id === "tray:decision:review") {
        openTarget();
        return;
      }
      if (id !== "tray:decision:allow" && id !== "tray:decision:deny") return;
      void answerTrayDecision(target, id === "tray:decision:allow", {
        appLocked: isAppLockEngaged(),
        currentPromptSeq: (sessionId) =>
          target?.serverId === activeId && snap?.session_id === sessionId && snap.permission_prompt != null
            ? snap.prompt_seq
            : null,
        resolveBaseUrl: (serverId) => stored.find((server) => server.id === serverId)?.baseUrl ?? null,
        answer: answerPermission,
      }).then((result) => {
        if (result === "answered") return;
        // Locked: bring the app up to its unlock gate and go nowhere. Anything else (prompt
        // changed, server gone, daemon refused): show the user the session as it is now.
        if (result === "locked") void openMainWindow();
        else openTarget();
      });
    };
    return () => {
      decisionHandler = null;
    };
  }, []);

  useEffect(() => {
    if (!isTauri) return;
    const rows = sessions ?? [];
    const waiting = rows.filter((row) => row.waiting);
    const summary: TraySummary = {
      busy: rows.filter((row) => row.busy && !row.waiting).length,
      waiting: waiting.length,
      costUsd: rows.reduce((total, row) => total + (row.cost_usd || 0), 0) || undefined,
      decision,
      sessions: rows.slice(0, TRAY_SESSION_LIMIT).map((row) => ({
        id: row.id,
        title: redactTrayTitle(row.title, locked),
        state: row.waiting ? "waiting" : row.busy ? "busy" : "idle",
        meta: formatRelativeTime(row.last_activity),
      })),
    };
    // The fleet query re-resolves on a timer; only pay for the IPC when something visible moved.
    const encoded = JSON.stringify(summary);
    if (encoded === lastPushed.current) return;
    lastPushed.current = encoded;
    void pushTraySummary(summary);
  }, [sessions, locked, decision]);
}

/** Tray decision rows are answered here rather than through the menu registry: they carry the
 * exact prompt they were built for, which a plain `() => void` handler cannot receive. */
let decisionHandler: ((id: string, target: TrayDecisionTarget | null) => void) | null = null;

async function openMainWindow(): Promise<void> {
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke("tray_open_main");
  } catch {
    // The window is already up whenever this runs from an in-app path.
  }
}

/**
 * Subscribes to the shell's menu/tray events for the lifetime of the app. Mounted once, from
 * `useGlobalShortcuts` (the app root). No-op on web/iOS/Android.
 */
export function useDesktopMenu(): void {
  useEffect(() => {
    if (!isTauri) return;
    let unlisten: (() => void) | null = null;
    let active = true;

    void (async () => {
      const { listen } = await import("@tauri-apps/api/event");
      const stop = await listen<{ id: string; decision?: TrayDecisionTarget }>(MENU_EVENT, (event) => {
        const { id, decision } = event.payload;
        if (id.startsWith("tray:decision:")) {
          if (decisionHandler) decisionHandler(id, decision ?? null);
          else router.push("/inbox");
          return;
        }
        dispatch(id);
      });
      if (active) unlisten = stop;
      else stop();
    })();

    // App-level About facts are context-free, so they can be pushed from here; the daemon line
    // is filled in by whoever holds the paired-server state.
    void (async () => {
      const { getVersion } = await import("@tauri-apps/api/app");
      await pushAboutInfo({ appVersion: await getVersion(), protocol: `v${PROTOCOL_VERSION}` });
    })().catch(() => undefined);

    return () => {
      active = false;
      unlisten?.();
    };
  }, []);
}
