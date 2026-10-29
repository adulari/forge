import AsyncStorage from "@react-native-async-storage/async-storage";
import { useSyncExternalStore } from "react";

import { isTauri } from "../platform";
import { syncGlobalQuickComposer } from "./globalChord";
import { whenDisjoint } from "./when";

export type AppShortcutAction =
  | "nav.fleet"
  | "nav.inbox"
  | "nav.history"
  | "nav.settings"
  | "session.new"
  | "command.palette"
  | "session.search"
  | "find"
  | "shell.sidebar"
  | "shell.usage"
  | "shell.quickComposer"
  | "workbench.split"
  | "workbench.terminal"
  | "workbench.gitReview"
  | "workbench.closePane"
  | "workbench.maximize"
  | "workbench.paneNext"
  | "workbench.panePrev"
  | "workbench.tasks"
  | "workbench.agents"
  | "workbench.plan"
  | "workbench.files"
  | "workbench.preview"
  | "session.next"
  | "session.prev"
  | "session.recent"
  | "session.close"
  | "settings"
  | "composer.effort"
  | "composer.temper"
  | "composer.model"
  | "composer.plan"
  | "sidebar.archive"
  | "sidebar.rename"
  | "session.chat"
  | "session.tasks"
  | "session.agents"
  | "session.review"
  | "session.focusComposer"
  | "session.interrupt";

export type AppShortcutGroup = "Navigation" | "Workbench" | "Session" | "Composer" | "Sidebar";

export interface ShortcutBinding {
  key: string;
  mod: boolean;
  alt: boolean;
  shift: boolean;
}

export interface AppShortcutDescriptor {
  action: AppShortcutAction;
  label: string;
  description: string;
  group: AppShortcutGroup;
  defaultBinding: ShortcutBinding;
  /** Focus-tag expression (`composerFocus && !terminalFocus`); the chord only fires, and only conflicts, inside it. */
  when?: string;
}

/** A chord handled locally by a field or overlay: listed for reference, never rebound. */
export interface DocShortcutRow {
  label: string;
  keys: string;
  group: AppShortcutGroup;
}

export const DOC_SHORTCUTS: readonly DocShortcutRow[] = [
  { label: "Send message", keys: "Enter", group: "Composer" },
  { label: "New line", keys: "Shift+Enter", group: "Composer" },
  { label: "Voice input", keys: "Hold the mic button", group: "Composer" },
  { label: "Dismiss overlay or stop dictation", keys: "Esc", group: "Navigation" },
  { label: "Move through palette results", keys: "Up / Down", group: "Navigation" },
  { label: "Run palette result", keys: "Enter", group: "Navigation" },
];

const binding = (
  key: string,
  modifiers: Partial<Pick<ShortcutBinding, "mod" | "alt" | "shift">>,
): ShortcutBinding => ({
  key,
  mod: modifiers.mod ?? false,
  alt: modifiers.alt ?? false,
  shift: modifiers.shift ?? false,
});

export const APP_SHORTCUTS: readonly AppShortcutDescriptor[] = [
  { action: "nav.fleet", label: "Open Fleet", description: "Go to the fleet overview.", group: "Navigation", defaultBinding: binding("1", { alt: true }) },
  { action: "nav.inbox", label: "Open Inbox", description: "Go to waiting decisions.", group: "Navigation", defaultBinding: binding("2", { alt: true }) },
  { action: "nav.history", label: "Open History", description: "Go to session history.", group: "Navigation", defaultBinding: binding("3", { alt: true }) },
  { action: "nav.settings", label: "Open Settings", description: "Go to app settings.", group: "Navigation", defaultBinding: binding("4", { alt: true }) },
  { action: "session.new", label: "New session", description: "Open the new-session flow.", group: "Navigation", defaultBinding: binding("n", { mod: true }) },
  { action: "command.palette", label: "Command palette", description: "Open actions and navigation.", group: "Navigation", defaultBinding: binding("k", { mod: true }) },
  { action: "session.search", label: "Search sessions", description: "Open global thread search.", group: "Navigation", defaultBinding: binding("p", { mod: true }) },
  { action: "find", label: "Find", description: "Search the transcript, diff or file you are looking at.", group: "Navigation", defaultBinding: binding("f", { mod: true }) },
  { action: "shell.sidebar", label: "Toggle sidebar", description: "Collapse or expand the desktop sidebar.", group: "Workbench", defaultBinding: binding("\\", { mod: true }) },
  { action: "shell.usage", label: "Toggle usage panel", description: "Show or hide the usage dock.", group: "Workbench", defaultBinding: binding("u", { mod: true }) },
  { action: "shell.quickComposer", label: "Quick composer", description: "Open the quick composer, from any app on desktop.", group: "Workbench", defaultBinding: binding("space", { alt: true }) },
  { action: "workbench.split", label: "Split pane", description: "Open another session in a new pane beside this one.", group: "Workbench", defaultBinding: binding("d", { mod: true }) },
  { action: "workbench.terminal", label: "Toggle terminal", description: "Open or close the terminal dock.", group: "Workbench", defaultBinding: binding("j", { mod: true }) },
  { action: "workbench.gitReview", label: "Toggle Git review", description: "Open or close working-tree review.", group: "Workbench", defaultBinding: binding("g", { mod: true }) },
  { action: "workbench.closePane", label: "Close pane", description: "Close the focused pane, or the last one when only one is split off.", group: "Workbench", defaultBinding: binding("d", { mod: true, shift: true }) },
  { action: "workbench.maximize", label: "Maximize pane", description: "Show the focused pane alone, or restore the grid.", group: "Workbench", defaultBinding: binding("m", { mod: true, shift: true }) },
  { action: "workbench.paneNext", label: "Next pane", description: "Move focus to the next pane.", group: "Workbench", defaultBinding: binding("]", { mod: true }) },
  { action: "workbench.panePrev", label: "Previous pane", description: "Move focus to the previous pane.", group: "Workbench", defaultBinding: binding("[", { mod: true }) },
  { action: "workbench.tasks", label: "Toggle Tasks", description: "Show or hide the task list of the focused session.", group: "Workbench", defaultBinding: binding("t", { mod: true, shift: true }) },
  { action: "workbench.agents", label: "Toggle Agents", description: "Show or hide the subagents of the focused session.", group: "Workbench", defaultBinding: binding("a", { mod: true, shift: true }) },
  { action: "workbench.plan", label: "Toggle Plan", description: "Show or hide the plan of the focused session.", group: "Workbench", defaultBinding: binding("l", { mod: true, shift: true }) },
  { action: "workbench.files", label: "Toggle Files", description: "Show or hide the workspace file browser.", group: "Workbench", defaultBinding: binding("f", { mod: true, shift: true }) },
  { action: "workbench.preview", label: "Toggle Preview", description: "Show or hide the browser preview of the focused session.", group: "Workbench", defaultBinding: binding("b", { mod: true, shift: true }) },
  { action: "settings", label: "Open app settings", description: "The standard preferences chord; opens app settings.", group: "Navigation", defaultBinding: binding(",", { mod: true }) },
  { action: "session.next", label: "Next session", description: "Open the next session in the list.", group: "Session", defaultBinding: binding("]", { mod: true, shift: true }) },
  { action: "session.prev", label: "Previous session", description: "Open the previous session in the list.", group: "Session", defaultBinding: binding("[", { mod: true, shift: true }) },
  { action: "session.recent", label: "Recent session", description: "Cycle through recently used sessions. Hold the modifier and press again to go further back.", group: "Session", defaultBinding: binding("tab", { mod: true }) },
  { action: "session.close", label: "Close session", description: "Close the focused pane, or leave the session when it is the only one.", group: "Session", defaultBinding: binding("w", { mod: true }), when: "hasSession && !composerFocus" },
  { action: "composer.effort", label: "Change effort", description: "Open the reasoning-effort picker.", group: "Composer", defaultBinding: binding("e", { alt: true }), when: "composerFocus" },
  { action: "composer.temper", label: "Change mode", description: "Open the operating-mode picker.", group: "Composer", defaultBinding: binding("t", { alt: true, shift: true }), when: "composerFocus" },
  { action: "composer.model", label: "Change model", description: "Open the model picker.", group: "Composer", defaultBinding: binding("m", { alt: true }), when: "composerFocus" },
  { action: "composer.plan", label: "Plan mode", description: "Start the draft with /plan.", group: "Composer", defaultBinding: binding("p", { alt: true }), when: "composerFocus" },
  { action: "sidebar.archive", label: "Archive session", description: "Archive the session under focus in the sidebar.", group: "Sidebar", defaultBinding: binding("e", {}), when: "sidebarFocus" },
  { action: "sidebar.rename", label: "Rename session", description: "Rename the session under focus in the sidebar.", group: "Sidebar", defaultBinding: binding("r", {}), when: "sidebarFocus" },
  { action: "session.chat", label: "Session: Chat", description: "Switch the current session to Chat.", group: "Session", defaultBinding: binding("c", { alt: true }) },
  { action: "session.tasks", label: "Session: Tasks", description: "Switch the current session to Tasks.", group: "Session", defaultBinding: binding("t", { alt: true }) },
  { action: "session.agents", label: "Session: Agents", description: "Switch the current session to Agents.", group: "Session", defaultBinding: binding("a", { alt: true }) },
  { action: "session.review", label: "Session: Review", description: "Switch the current session to Review.", group: "Session", defaultBinding: binding("r", { alt: true }) },
  { action: "session.focusComposer", label: "Focus composer", description: "Move focus to the prompt composer.", group: "Session", defaultBinding: binding("e", { mod: true }) },
  { action: "session.interrupt", label: "Interrupt session", description: "Stop the active turn.", group: "Session", defaultBinding: binding(".", { mod: true }) },
] as const;

const ACTIONS = new Set<AppShortcutAction>(APP_SHORTCUTS.map(({ action }) => action));
const DESCRIPTORS = new Map(APP_SHORTCUTS.map((descriptor) => [descriptor.action, descriptor]));
const STORAGE_KEY = "forge.appShortcuts.v1";

export type ShortcutOverrides = Partial<Record<AppShortcutAction, ShortcutBinding>> & Record<string, ShortcutBinding>;

interface ShortcutState {
  loaded: boolean;
  overrides: ShortcutOverrides;
}

let state: ShortcutState = { loaded: false, overrides: {} };
let hydration: Promise<void> | null = null;
let persistQueue: Promise<void> = Promise.resolve();
let nativeSyncQueue: Promise<void> = Promise.resolve();
const listeners = new Set<() => void>();

function emit(): void {
  for (const listener of listeners) listener();
}

function replaceState(next: ShortcutState): void {
  state = next;
  emit();
}

export function normalizeShortcutKey(rawKey: string): string | null {
  const raw = rawKey.length === 1 ? rawKey : rawKey.toLowerCase();
  const shifted: Record<string, string> = {
    "!": "1",
    "@": "2",
    "#": "3",
    "$": "4",
    "%": "5",
    "^": "6",
    "&": "7",
    "*": "8",
    "(": "9",
    ")": "0",
    "_": "-",
    "+": "=",
    "{": "[",
    "}": "]",
    ":": ";",
    "\"": "'",
    "|": "\\",
    "<": ",",
    ">": ".",
    "?": "/",
    "~": "`",
  };
  const key = shifted[raw] ?? raw.toLowerCase();
  if (key === " " || key === "spacebar") return "space";
  if (key === "control" || key === "meta" || key === "alt" || key === "shift") return null;
  if (/^[a-z0-9]$/.test(key) || /^[fF](?:[1-9]|1[0-9]|2[0-4])$/.test(key)) return key.toLowerCase();
  if (
    [
      "\\", "[", "]", ",", "=", "-", ".", "'", ";", "/", "`",
      "backspace", "enter", "space", "tab", "delete", "end", "home", "insert",
      "pagedown", "pageup", "arrowdown", "arrowleft", "arrowright", "arrowup",
    ].includes(key)
  ) {
    return key;
  }
  return null;
}

export interface ShortcutKeyEvent {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

export function shortcutBindingFromEvent(event: ShortcutKeyEvent): ShortcutBinding | null {
  const key = normalizeShortcutKey(event.key);
  if (!key) return null;
  return {
    key,
    mod: event.metaKey || event.ctrlKey,
    alt: event.altKey,
    shift: event.shiftKey,
  };
}

export function shortcutSignature(value: ShortcutBinding): string {
  return `${value.mod ? "M" : "-"}${value.alt ? "A" : "-"}${value.shift ? "S" : "-"}:${value.key}`;
}

export function shortcutBindingsEqual(left: ShortcutBinding, right: ShortcutBinding): boolean {
  return shortcutSignature(left) === shortcutSignature(right);
}

function isShortcutBinding(value: unknown): value is ShortcutBinding {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<ShortcutBinding>;
  return (
    typeof candidate.key === "string"
    && normalizeShortcutKey(candidate.key) === candidate.key
    && typeof candidate.mod === "boolean"
    && typeof candidate.alt === "boolean"
    && typeof candidate.shift === "boolean"
  );
}

export function parseShortcutOverrides(raw: string | null): ShortcutOverrides {
  if (!raw) return {};
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    const valid: ShortcutOverrides = {};
    for (const [action, value] of Object.entries(parsed)) {
      // Actions a newer build added stay in the file, inert here, so an older client never erases them.
      if (isShortcutBinding(value)) valid[action] = value;
    }
    return valid;
  } catch {
    return {};
  }
}

export function getDefaultShortcut(action: AppShortcutAction): ShortcutBinding {
  const descriptor = DESCRIPTORS.get(action);
  if (!descriptor) throw new Error(`unknown app shortcut: ${action}`);
  return descriptor.defaultBinding;
}

export function getResolvedShortcut(
  action: AppShortcutAction,
  overrides: ShortcutOverrides = state.overrides,
): ShortcutBinding {
  return overrides[action] ?? getDefaultShortcut(action);
}

export function isDefaultShortcut(action: AppShortcutAction, value = getResolvedShortcut(action)): boolean {
  return shortcutBindingsEqual(value, getDefaultShortcut(action));
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  void ensureShortcutPreferencesLoaded();
  return () => listeners.delete(listener);
}

function snapshot(): ShortcutState {
  return state;
}

export function useShortcutPreferences(): ShortcutState {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

export function useShortcutBinding(action: AppShortcutAction): ShortcutBinding {
  const { overrides } = useShortcutPreferences();
  return getResolvedShortcut(action, overrides);
}

function schedulePersist(overrides: ShortcutOverrides): Promise<void> {
  const serialized = JSON.stringify(overrides);
  persistQueue = persistQueue
    .catch(() => undefined)
    .then(() => AsyncStorage.setItem(STORAGE_KEY, serialized));
  return persistQueue;
}

function scheduleNativeSync(overrides: ShortcutOverrides): Promise<void> {
  nativeSyncQueue = nativeSyncQueue
    .catch(() => undefined)
    .then(() => syncNativeMenuShortcuts(overrides));
  return nativeSyncQueue;
}

export async function ensureShortcutPreferencesLoaded(): Promise<void> {
  if (state.loaded) return;
  if (hydration) return hydration;
  hydration = AsyncStorage.getItem(STORAGE_KEY)
    .then((raw) => {
      if (!state.loaded) {
        replaceState({ loaded: true, overrides: parseShortcutOverrides(raw) });
      }
      return scheduleNativeSync(state.overrides);
    })
    .catch(() => {
      if (!state.loaded) replaceState({ loaded: true, overrides: {} });
    });
  return hydration;
}

export async function setAppShortcut(action: AppShortcutAction, value: ShortcutBinding): Promise<void> {
  if (!ACTIONS.has(action) || !isShortcutBinding(value)) throw new Error("invalid app shortcut");
  await ensureShortcutPreferencesLoaded();
  const overrides = { ...state.overrides };
  if (isDefaultShortcut(action, value)) delete overrides[action];
  else overrides[action] = value;
  await commitOverrides(overrides);
}

export async function resetAppShortcut(action: AppShortcutAction): Promise<void> {
  await ensureShortcutPreferencesLoaded();
  if (!(action in state.overrides)) return;
  const overrides = { ...state.overrides };
  delete overrides[action];
  await commitOverrides(overrides);
}

/** Adopt overrides read from the daemon's keybindings.json; unknown actions are kept inertly. */
export async function adoptRemoteShortcuts(overrides: ShortcutOverrides): Promise<void> {
  await ensureShortcutPreferencesLoaded();
  await commitOverrides(overrides);
}

export async function resetAllAppShortcuts(): Promise<void> {
  await ensureShortcutPreferencesLoaded();
  await commitOverrides({});
}

async function commitOverrides(overrides: ShortcutOverrides): Promise<void> {
  const previous = state.overrides;
  replaceState({ loaded: true, overrides });
  try {
    await Promise.all([schedulePersist(overrides), scheduleNativeSync(overrides)]);
  } catch (error) {
    // Roll back only if another edit has not already superseded this one. Both
    // persistence channels are then reconciled to the currently visible state.
    if (state.overrides === overrides) replaceState({ loaded: true, overrides: previous });
    await Promise.allSettled([
      schedulePersist(state.overrides),
      scheduleNativeSync(state.overrides),
    ]);
    throw error;
  }
}

function isMacPlatform(): boolean {
  if (typeof navigator === "undefined") return false;
  return /mac|iphone|ipad|ipod/i.test(navigator.platform);
}

const KEY_LABELS: Record<string, string> = {
  space: "Space",
  arrowup: "↑",
  arrowdown: "↓",
  arrowleft: "←",
  arrowright: "→",
  backspace: "Backspace",
  delete: "Delete",
  enter: "Enter",
  tab: "Tab",
  pageup: "Page Up",
  pagedown: "Page Down",
};

export function formatShortcut(value: ShortcutBinding, mac = isMacPlatform()): string {
  const parts: string[] = [];
  if (value.mod) parts.push(mac ? "⌘" : "Ctrl");
  if (value.alt) parts.push(mac ? "⌥" : "Alt");
  if (value.shift) parts.push(mac ? "⇧" : "Shift");
  parts.push(KEY_LABELS[value.key] ?? (value.key.length === 1 ? value.key.toUpperCase() : value.key));
  return mac ? parts.join("") : parts.join("+");
}

/** The user's current binding for `action`, formatted for a keycap hint or an a11y hint. Hints
 * read the resolved binding rather than a hard-coded key so a rebinding (and the Ctrl-vs-⌘
 * platform difference) shows up everywhere the shortcut is advertised. */
export function useShortcutLabel(action: AppShortcutAction): string {
  return formatShortcut(useShortcutBinding(action));
}

interface ReservedShortcut {
  id: string;
  label: string;
  binding: ShortcutBinding;
}

const RESERVED_SHORTCUTS: readonly ReservedShortcut[] = [
  { id: "standard.undo", label: "standard Undo", binding: binding("z", { mod: true }) },
  { id: "standard.redo", label: "standard Redo", binding: binding("z", { mod: true, shift: true }) },
  { id: "standard.cut", label: "standard Cut", binding: binding("x", { mod: true }) },
  { id: "standard.copy", label: "standard Copy", binding: binding("c", { mod: true }) },
  { id: "standard.paste", label: "standard Paste", binding: binding("v", { mod: true }) },
  { id: "standard.selectAll", label: "standard Select All", binding: binding("a", { mod: true }) },
  { id: "standard.quit", label: "standard Quit", binding: binding("q", { mod: true }) },
  { id: "standard.approve", label: "native Approve Waiting Decision", binding: binding("enter", { mod: true }) },
  { id: "standard.checkpoint", label: "native Create Checkpoint", binding: binding("s", { mod: true }) },
] as const;

export interface ShortcutConflict {
  id: string;
  label: string;
  kind: "app" | "reserved";
}

export function findShortcutConflicts(
  action: AppShortcutAction,
  value: ShortcutBinding,
  overrides: ShortcutOverrides = state.overrides,
): ShortcutConflict[] {
  const signature = shortcutSignature(value);
  const own = DESCRIPTORS.get(action)?.when;
  const app = APP_SHORTCUTS
    .filter((descriptor) =>
      descriptor.action !== action
      && shortcutSignature(getResolvedShortcut(descriptor.action, overrides)) === signature
      && !whenDisjoint(own, descriptor.when))
    .map((descriptor) => ({ id: descriptor.action, label: descriptor.label, kind: "app" as const }));
  const reserved = RESERVED_SHORTCUTS
    .filter((item) => shortcutSignature(item.binding) === signature)
    .map((item) => ({ id: item.id, label: item.label, kind: "reserved" as const }));
  return [...app, ...reserved];
}

const NATIVE_MENU_ACTIONS: Partial<Record<AppShortcutAction, string>> = {
  "session.new": "session:new",
  "session.search": "session:search",
  "shell.sidebar": "view:sidebar",
  "shell.usage": "view:usage",
  "shell.quickComposer": "session:quick-composer",
  "workbench.split": "view:split-pane",
  "workbench.terminal": "view:terminal",
  "workbench.gitReview": "view:git-review",
  "workbench.closePane": "view:pane-close",
  "workbench.maximize": "view:pane-maximize",
  "workbench.paneNext": "view:pane-next",
  "workbench.panePrev": "view:pane-prev",
  "workbench.tasks": "view:tasks",
  "workbench.agents": "view:agents",
  "workbench.plan": "view:plan",
  "workbench.files": "view:files",
  "workbench.preview": "view:browser-preview",
  "session.interrupt": "session:interrupt",
};

function tauriAccelerator(value: ShortcutBinding): string {
  const parts: string[] = [];
  if (value.mod) parts.push("CmdOrCtrl");
  if (value.alt) parts.push("Alt");
  if (value.shift) parts.push("Shift");
  const names: Record<string, string> = {
    "\\": "Backslash",
    "[": "BracketLeft",
    "]": "BracketRight",
    ",": "Comma",
    "=": "Equal",
    "-": "Minus",
    ".": "Period",
    "'": "Quote",
    ";": "Semicolon",
    "/": "Slash",
    "`": "Backquote",
    arrowdown: "ArrowDown",
    arrowleft: "ArrowLeft",
    arrowright: "ArrowRight",
    arrowup: "ArrowUp",
    backspace: "Backspace",
    delete: "Delete",
    end: "End",
    enter: "Enter",
    home: "Home",
    insert: "Insert",
    pagedown: "PageDown",
    pageup: "PageUp",
    space: "Space",
    tab: "Tab",
  };
  parts.push(names[value.key] ?? value.key.toUpperCase());
  return parts.join("+");
}

async function syncNativeMenuShortcuts(overrides: ShortcutOverrides): Promise<void> {
  if (!isTauri) return;
  // First, and it never rejects: a chord another app owns must not roll back the saved binding.
  await syncGlobalQuickComposer(tauriAccelerator(getResolvedShortcut("shell.quickComposer", overrides)));
  const accelerators = Object.entries(NATIVE_MENU_ACTIONS).map(([action, id]) => ({
    id,
    // A duplicate native key equivalent would be swallowed by whichever macOS menu
    // item registered first. Removing the menu equivalent lets the webview registry
    // deliver the chord to every conflicting app action, matching the warning shown
    // in Settings. Reserved OS/menu conflicts remain warnings because Forge cannot
    // safely remove standard Cut/Copy/Quit/etc. accelerators.
    accelerator:
      findShortcutConflicts(
        action as AppShortcutAction,
        getResolvedShortcut(action as AppShortcutAction, overrides),
        overrides,
      ).length > 0
        ? null
        : tauriAccelerator(getResolvedShortcut(action as AppShortcutAction, overrides)),
  }));
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("set_menu_accelerators", { accelerators });
}
