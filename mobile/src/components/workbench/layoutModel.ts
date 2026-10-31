// Pure model for the desktop pane grid (several sessions on one screen) plus the persisted dock
// sizes. Pane 0 is never stored: it IS the routed session (see components/shell/SplitPanes.tsx),
// so only the *secondary* slots live here. A slot is a session id, or `null` for an empty pane
// that shows the session picker.

export const MAX_PANES = 4;
/** Narrowest a pane may get before the grid hides panes instead of squeezing them. */
export const MIN_PANE_WIDTH = 400;
export const RIGHT_DOCK_MIN = 240;
export const RIGHT_DOCK_MAX = 760;
export const BOTTOM_DOCK_MIN = 110;
export const BOTTOM_DOCK_MAX = 560;
/** Room that must stay free for the content column when a right dock widens. */
export const MIN_CONTENT_WIDTH = 480;

export type PaneSlot = string | null;
export type LayoutKind = "single" | "cols2" | "cols3" | "grid";
export type PresetName = "focus" | "side-by-side" | "grid" | "review";

export interface PaneWeights {
  /** Column weights for two panes (sum 1). */
  cols2: number[];
  /** Column weights for three panes (sum 1). */
  cols3: number[];
  /** `[left column share, top row share]` for the 2x2 grid. */
  grid: number[];
}

export interface DockSizes {
  /** One width for the whole right lane, so switching tabs never makes the column jump. */
  right: number | null;
  bottom: number | null;
}

export interface PaneLayoutState {
  secondary: PaneSlot[];
  /**
   * In-memory identity of each secondary pane, parallel to `secondary` (never persisted). The view
   * keys panes by it, so closing a pane does not hand its neighbour's React element (and live
   * socket) to another session. Ids are 1-based; 0 means "pane 0" in drag payloads.
   */
  slotIds: number[];
  nextSlotId: number;
  /** Index into `[primary, ...secondary]`. */
  focus: number;
  /** A pane shown alone while the others stay mounted but hidden. */
  maximized: number | null;
  weights: PaneWeights;
  dock: DockSizes;
}

export const DEFAULT_WEIGHTS: PaneWeights = {
  cols2: [0.5, 0.5],
  cols3: [1 / 3, 1 / 3, 1 / 3],
  grid: [0.5, 0.5],
};

export function initialLayout(): PaneLayoutState {
  return {
    secondary: [],
    slotIds: [],
    nextSlotId: 1,
    focus: 0,
    maximized: null,
    weights: { cols2: [...DEFAULT_WEIGHTS.cols2], cols3: [...DEFAULT_WEIGHTS.cols3], grid: [...DEFAULT_WEIGHTS.grid] },
    dock: { right: null, bottom: null },
  };
}

export function paneCount(state: PaneLayoutState): number {
  return 1 + state.secondary.length;
}

export function layoutKind(count: number): LayoutKind {
  if (count <= 1) return "single";
  if (count === 2) return "cols2";
  if (count === 3) return "cols3";
  return "grid";
}

/**
 * How many of `total` panes fit a container `width` wide at MIN_PANE_WIDTH each. The 2x2 grid
 * needs two columns; below that only the primary pane shows. Hidden panes keep their state.
 */
export function visiblePaneCount(total: number, width: number): number {
  if (total <= 1 || width <= 0) return Math.max(1, Math.min(total, 1));
  const columns = Math.max(1, Math.floor(width / MIN_PANE_WIDTH));
  if (total >= 4) return columns >= 2 ? 4 : 1;
  return Math.min(total, columns);
}

/**
 * Replaces the whole secondary list. A slot that already shows the same session keeps its id, so
 * a preset that keeps a pane does not remount it; everything else gets a fresh id.
 */
export function withSecondary(state: PaneLayoutState, secondary: PaneSlot[]): PaneLayoutState {
  const spare = new Map<string, number[]>();
  state.secondary.forEach((slot, i) => {
    const id = state.slotIds[i];
    if (slot != null && id != null) spare.set(slot, [...(spare.get(slot) ?? []), id]);
  });
  let next = state.nextSlotId;
  const slotIds = secondary.map((slot) => (slot != null ? spare.get(slot)?.shift() : undefined) ?? next++);
  return { ...state, secondary, slotIds, nextSlotId: next };
}

/**
 * Secondary panes in the order the view renders them: ascending slot id, keyed by it. Panes are
 * absolutely positioned, so DOM order carries no meaning; a stable order means closing or
 * inserting a pane never asks React to move a live node (which drops scroll and focus on web).
 */
export function secondaryRenderOrder(state: PaneLayoutState): { slot: PaneSlot; index: number; slotId: number }[] {
  return state.secondary
    .map((slot, i) => ({ slot, index: i + 1, slotId: state.slotIds[i] ?? 0 }))
    .sort((a, b) => a.slotId - b.slotId);
}

/** Current pane index (>= 1) of secondary slot id `slot`; 0 maps to pane 0; -1 when it is gone. */
export function indexOfSlotId(state: PaneLayoutState, slot: number): number {
  if (slot === 0) return 0;
  const at = state.slotIds.indexOf(slot);
  return at < 0 ? -1 : at + 1;
}

export interface PaneSize {
  width: number;
  height: number;
}

/**
 * The grid's size after an `onLayout`. A collapsed container (an expanding dock, a hidden route)
 * reports 0 on an axis: keep the last real size, otherwise `visiblePaneCount` drops to a single
 * pane and focus jumps to pane 0.
 */
export function nextPaneSize(current: PaneSize | null, width: number, height: number): PaneSize | null {
  if (width === 0 || height === 0) return current;
  return current && current.width === width && current.height === height ? current : { width, height };
}

function clampIndex(state: PaneLayoutState, index: number): number {
  return Math.max(0, Math.min(paneCount(state) - 1, index));
}

export function focusPane(state: PaneLayoutState, index: number): PaneLayoutState {
  const focus = clampIndex(state, index);
  return focus === state.focus ? state : { ...state, focus };
}

export function cycleFocus(state: PaneLayoutState, direction: 1 | -1): PaneLayoutState {
  const total = paneCount(state);
  if (total <= 1) return state;
  return { ...state, focus: (state.focus + direction + total) % total };
}

export function toggleMaximize(state: PaneLayoutState, index: number): PaneLayoutState {
  if (paneCount(state) <= 1) return state;
  const target = clampIndex(state, index);
  return { ...state, focus: target, maximized: state.maximized === target ? null : target };
}

/** Appends a pane (empty when `slot` is null) and focuses it. A full grid is returned unchanged. */
export function addPane(state: PaneLayoutState, slot: PaneSlot): PaneLayoutState {
  if (paneCount(state) >= MAX_PANES) return state;
  return {
    ...state,
    secondary: [...state.secondary, slot],
    slotIds: [...state.slotIds, state.nextSlotId],
    nextSlotId: state.nextSlotId + 1,
    focus: state.secondary.length + 1,
    maximized: null,
  };
}

export function setSlot(state: PaneLayoutState, index: number, slot: PaneSlot): PaneLayoutState {
  if (index < 1 || index > state.secondary.length) return state;
  return { ...state, secondary: state.secondary.map((current, i) => (i === index - 1 ? slot : current)) };
}

/** Removes secondary pane `index` (>= 1). Pane 0 is closed through `promoteFirstSecondary`. */
export function removePane(state: PaneLayoutState, index: number): PaneLayoutState {
  if (index < 1 || index > state.secondary.length) return state;
  const secondary = state.secondary.filter((_, i) => i !== index - 1);
  const slotIds = state.slotIds.filter((_, i) => i !== index - 1);
  let focus = state.focus;
  if (focus === index) focus = index - 1;
  else if (focus > index) focus -= 1;
  const maximized = state.maximized === index ? null : state.maximized != null && state.maximized > index ? state.maximized - 1 : state.maximized;
  return { ...state, secondary, slotIds, focus, maximized: secondary.length === 0 ? null : maximized };
}

/** Closing pane 0: the first secondary session becomes the routed one. */
export function promoteFirstSecondary(state: PaneLayoutState): { state: PaneLayoutState; promote: string | null } {
  if (state.secondary.length === 0) return { state, promote: null };
  const [first, ...rest] = state.secondary;
  const next: PaneLayoutState = {
    ...state,
    secondary: rest,
    slotIds: state.slotIds.slice(1),
    focus: 0,
    maximized: rest.length === 0 ? null : state.maximized === 0 ? 0 : null,
  };
  return { state: next, promote: first };
}

/**
 * Open `id` in the focused pane. Already-open sessions just take focus (a session is never
 * shown twice). Focus on pane 0 means the route changes, reported as `routeTo`.
 */
export function openInFocused(
  state: PaneLayoutState,
  primary: string | null,
  id: string,
): { state: PaneLayoutState; routeTo: string | null } {
  if (id === primary) return { state: focusPane(state, 0), routeTo: null };
  const existing = state.secondary.indexOf(id);
  if (existing >= 0) return { state: focusPane(state, existing + 1), routeTo: null };
  if (state.focus >= 1 && state.focus <= state.secondary.length) {
    return { state: setSlot(state, state.focus, id), routeTo: null };
  }
  return { state, routeTo: id };
}

/** Open `id` in a new pane, reusing the last pane when the grid is full. */
export function openInNewPane(state: PaneLayoutState, primary: string | null, id: string): PaneLayoutState {
  if (id === primary) return focusPane(state, 0);
  const existing = state.secondary.indexOf(id);
  if (existing >= 0) return focusPane(state, existing + 1);
  if (paneCount(state) < MAX_PANES) return addPane(state, id);
  const last = state.secondary.length;
  return { ...setSlot(state, last, id), focus: last };
}

/**
 * A tap on a session link inside secondary pane `index` (an agent row, the subagent strip, Fork).
 * Unlike a drop it never moves or duplicates anything: a session already on screen just takes
 * focus, and only an unopened one replaces this pane's session.
 */
export function openFromPane(
  state: PaneLayoutState,
  primary: string | null,
  index: number,
  id: string,
): PaneLayoutState {
  if (id === primary) return focusPane(state, 0);
  const existing = state.secondary.indexOf(id);
  if (existing >= 0) return focusPane(state, existing + 1);
  return { ...setSlot(state, index, id), focus: index };
}

/** Drop `id` onto pane `index`. Dropping on pane 0 changes the route. */
export function openInPane(
  state: PaneLayoutState,
  primary: string | null,
  index: number,
  id: string,
): { state: PaneLayoutState; routeTo: string | null } {
  if (id === primary && index === 0) return { state: focusPane(state, 0), routeTo: null };
  const existing = state.secondary.indexOf(id);
  let next = state;
  // Dropping a session that is open elsewhere moves it: the vacated pane keeps its old slot's
  // neighbour order by becoming empty rather than shifting everything.
  if (existing >= 0 && existing + 1 !== index) next = setSlot(next, existing + 1, null);
  if (index === 0) return { state: focusPane(next, 0), routeTo: id };
  return { state: { ...setSlot(next, index, id), focus: index }, routeTo: null };
}

/** Swap secondary pane `index` with pane 0: the old primary takes the slot, `promote` is routed. */
export function swapWithPrimary(
  state: PaneLayoutState,
  primary: string | null,
  index: number,
): { state: PaneLayoutState; promote: string | null } {
  const slot = state.secondary[index - 1];
  if (index < 1 || slot === undefined) return { state, promote: null };
  return { state: setSlot(state, index, primary), promote: slot };
}

export function swapSecondary(state: PaneLayoutState, a: number, b: number): PaneLayoutState {
  if (a < 1 || b < 1 || a > state.secondary.length || b > state.secondary.length || a === b) return state;
  const secondary = [...state.secondary];
  [secondary[a - 1], secondary[b - 1]] = [secondary[b - 1] ?? null, secondary[a - 1] ?? null];
  const slotIds = [...state.slotIds];
  [slotIds[a - 1], slotIds[b - 1]] = [slotIds[b - 1] as number, slotIds[a - 1] as number];
  const focus = state.focus === a ? b : state.focus === b ? a : state.focus;
  return { ...state, secondary, slotIds, focus };
}

/** Smallest weight a pane may shrink to, as a share of the container, for `count` panes. */
export function minWeight(containerSize: number, count: number): number {
  if (containerSize <= 0) return 0.1;
  return Math.max(0.1, Math.min(1 / count, MIN_PANE_WIDTH / containerSize));
}

export function normalizeWeights(weights: number[], count: number): number[] {
  const valid = weights.length === count && weights.every((w) => Number.isFinite(w) && w > 0);
  if (!valid) return Array.from({ length: count }, () => 1 / count);
  const sum = weights.reduce((a, b) => a + b, 0);
  return weights.map((w) => w / sum);
}

/** Move the seam between weight `boundary` and `boundary + 1` by `delta` (a share), keeping both >= `min`. */
export function resizeBoundary(weights: number[], boundary: number, delta: number, min: number): number[] {
  if (boundary < 0 || boundary >= weights.length - 1) return weights;
  const left = weights[boundary] ?? 0;
  const right = weights[boundary + 1] ?? 0;
  const total = left + right;
  const nextLeft = Math.max(min, Math.min(total - min, left + delta));
  const next = [...weights];
  next[boundary] = nextLeft;
  next[boundary + 1] = total - nextLeft;
  return next;
}

export function clampFraction(value: number, min: number): number {
  return Math.max(min, Math.min(1 - min, value));
}

export function clampRightWidth(width: number, windowWidth: number, occupiedWidth: number): number {
  const max = Math.max(RIGHT_DOCK_MIN, Math.min(RIGHT_DOCK_MAX, windowWidth - occupiedWidth - MIN_CONTENT_WIDTH));
  return Math.round(Math.max(RIGHT_DOCK_MIN, Math.min(max, width)));
}

export function clampBottomHeight(height: number, windowHeight: number): number {
  const max = Math.max(BOTTOM_DOCK_MIN, Math.min(BOTTOM_DOCK_MAX, Math.round(windowHeight * 0.6)));
  return Math.round(Math.max(BOTTOM_DOCK_MIN, Math.min(max, height)));
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

export const LEGACY_SPLIT_KEY = "forge.splitPanes";
export const WORKBENCH_STORAGE_KEY = "forge.workbench.v2";

export interface PersistedLayout {
  secondary: PaneSlot[];
  weights: PaneWeights;
  dock: DockSizes;
}

export function serializeLayout(state: PaneLayoutState): PersistedLayout {
  return { secondary: state.secondary, weights: state.weights, dock: state.dock };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseNumberArray(value: unknown, count: number, fallback: number[]): number[] {
  if (!Array.isArray(value) || value.length !== count) return [...fallback];
  const numbers = value.filter((entry): entry is number => typeof entry === "number" && Number.isFinite(entry) && entry > 0 && entry < 1);
  return numbers.length === count ? normalizeWeights(numbers, count) : [...fallback];
}

/** Tolerant parse: anything malformed falls back to defaults instead of throwing. */
export function parseLayout(raw: unknown): PaneLayoutState {
  const base = initialLayout();
  if (!isRecord(raw)) return base;
  const secondary = Array.isArray(raw.secondary)
    ? raw.secondary.filter((slot): slot is PaneSlot => slot === null || typeof slot === "string").slice(0, MAX_PANES - 1)
    : [];
  const weights = isRecord(raw.weights) ? raw.weights : {};
  const grid = parseNumberArray(weights.grid, 2, DEFAULT_WEIGHTS.grid);
  const dock: DockSizes = { right: null, bottom: null };
  if (isRecord(raw.dock)) {
    if (typeof raw.dock.bottom === "number" && Number.isFinite(raw.dock.bottom)) dock.bottom = raw.dock.bottom;
    if (typeof raw.dock.right === "number" && Number.isFinite(raw.dock.right)) dock.right = raw.dock.right;
  }
  return {
    ...withSecondary(base, secondary),
    weights: {
      cols2: parseNumberArray(weights.cols2, 2, DEFAULT_WEIGHTS.cols2),
      cols3: parseNumberArray(weights.cols3, 3, DEFAULT_WEIGHTS.cols3),
      grid,
    },
    dock,
  };
}

/** The pre-workbench `forge.splitPanes` entry: `{ on, secondary }`. */
export function migrateLegacySplit(raw: string | null): PaneLayoutState | null {
  if (!raw) return null;
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!isRecord(parsed) || typeof parsed.on !== "boolean") return null;
    const base = initialLayout();
    if (!parsed.on) return base;
    return withSecondary(base, [typeof parsed.secondary === "string" ? parsed.secondary : null]);
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

export interface PaneRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface PaneSeam {
  key: keyof PaneWeights;
  orientation: "vertical" | "horizontal";
  /** Which weight the seam moves: the boundary between weight `boundary` and the next. */
  boundary: number;
  /** Pixel position of the seam along the axis it resizes. */
  at: number;
}

export interface PaneGeometry {
  kind: LayoutKind;
  /** Panes actually on screen (`null` rects are mounted but hidden). */
  visible: number;
  rects: (PaneRect | null)[];
  seams: PaneSeam[];
}

/** Raise every share under `floor` to it, taking the difference from the others pro rata. */
export function liftToFloor(shares: number[], floor: number): number[] {
  let result = [...shares];
  for (let pass = 0; pass < shares.length; pass += 1) {
    const low = result.map((share) => share < floor - 1e-9);
    const lowCount = low.filter(Boolean).length;
    if (lowCount === 0 || lowCount === result.length) break;
    const othersSum = result.reduce((sum, share, i) => (low[i] ? sum : sum + share), 0);
    const scale = (1 - floor * lowCount) / othersSum;
    result = result.map((share, i) => (low[i] ? floor : share * scale));
  }
  return result;
}

/** Share of the height a grid row may not shrink below. */
export const MIN_ROW_SHARE = 0.2;

/**
 * Where every pane goes inside a `width` x `height` container. Absolute rects (rather than
 * nested flex rows) keep pane 0 — the live route stack — at one stable place in the React tree
 * through every layout change, so adding or removing a pane never remounts the router.
 */
export function paneGeometry(
  total: number,
  width: number,
  height: number,
  weights: PaneWeights,
  maximized: number | null,
): PaneGeometry {
  const rects: (PaneRect | null)[] = Array.from({ length: Math.max(1, total) }, () => null);
  const full: PaneRect = { left: 0, top: 0, width, height };
  if (maximized != null && maximized >= 0 && maximized < rects.length) {
    rects[maximized] = full;
    return { kind: "single", visible: 1, rects, seams: [] };
  }
  const visible = visiblePaneCount(total, width);
  const kind = layoutKind(visible);
  if (kind === "single") {
    rects[0] = full;
    return { kind, visible: 1, rects, seams: [] };
  }
  if (kind === "grid") {
    const colShare = clampFraction(weights.grid[0] ?? 0.5, minWeight(width, 2));
    const rowShare = clampFraction(weights.grid[1] ?? 0.5, MIN_ROW_SHARE);
    const x = Math.round(width * colShare);
    const y = Math.round(height * rowShare);
    rects[0] = { left: 0, top: 0, width: x, height: y };
    rects[1] = { left: x, top: 0, width: width - x, height: y };
    rects[2] = { left: 0, top: y, width: x, height: height - y };
    rects[3] = { left: x, top: y, width: width - x, height: height - y };
    return {
      kind,
      visible,
      rects,
      seams: [
        { key: "grid", orientation: "vertical", boundary: 0, at: x },
        { key: "grid", orientation: "horizontal", boundary: 1, at: y },
      ],
    };
  }
  const key = kind === "cols2" ? "cols2" : "cols3";
  // Weights saved on a wider window can leave a pane under its minimum here: lift them back.
  const shares = liftToFloor(normalizeWeights(weights[key], visible), minWeight(width, visible));
  const seams: PaneSeam[] = [];
  let cursor = 0;
  shares.forEach((share, index) => {
    const isLast = index === visible - 1;
    const w = isLast ? width - cursor : Math.round(width * share);
    rects[index] = { left: cursor, top: 0, width: w, height };
    cursor += w;
    if (!isLast) seams.push({ key, orientation: "vertical", boundary: index, at: cursor });
  });
  return { kind, visible, rects, seams };
}
