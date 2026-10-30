// Pure view-model helpers for the Forge Anywhere device list. The relay returns only
// `name`, `created_at` and `last_seen_at` per device, so everything shown here is derived
// from those — enough to tell five "Forge on iPhone" enrollments apart and to spot the
// ones that were abandoned.
import type { AnywhereDevice } from "../anywhereApi";

export const STALE_AFTER_DAYS = 30;
const DAY_MS = 24 * 60 * 60 * 1000;
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

function parse(iso: string | null | undefined): number | null {
  if (!iso) return null;
  const ms = Date.parse(iso);
  return Number.isNaN(ms) ? null : ms;
}

export function formatShortDate(ms: number, nowMs: number): string {
  const date = new Date(ms);
  const base = `${MONTHS[date.getMonth()]} ${date.getDate()}`;
  return date.getFullYear() === new Date(nowMs).getFullYear() ? base : `${base}, ${date.getFullYear()}`;
}

export function formatAgo(ms: number, nowMs: number): string {
  const minutes = Math.floor(Math.max(0, nowMs - ms) / 60_000);
  if (minutes < 5) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 60) return `${days}d ago`;
  return formatShortDate(ms, nowMs);
}

export type DeviceKind = "phone" | "computer";

export function deviceKind(name: string): DeviceKind {
  return /phone|ipad|android|pixel|tablet|mobile|ios/i.test(name) ? "phone" : "computer";
}

export interface DeviceRowModel {
  id: string;
  name: string;
  kind: DeviceKind;
  isThisDevice: boolean;
  /** "Last seen 3d ago", "Active now" or "Never connected". */
  lastSeen: string;
  /** "Enrolled Oct 12". */
  enrolled: string;
  stale: boolean;
  /** Another enrollment shares this name, so the name alone cannot identify it. */
  duplicateName: boolean;
}

export function describeDevice(
  device: AnywhereDevice,
  ctx: { currentDeviceId: string | null | undefined; nowMs: number; nameCounts: ReadonlyMap<string, number> },
): DeviceRowModel {
  const { nowMs } = ctx;
  const created = parse(device.created_at);
  const seen = parse(device.last_seen_at);
  const isThisDevice = device.id === ctx.currentDeviceId;
  const reference = seen ?? created;
  const stale = !isThisDevice && reference != null && nowMs - reference > STALE_AFTER_DAYS * DAY_MS;
  const lastSeen = isThisDevice
    ? "Active now"
    : seen == null
      ? "Never connected"
      : nowMs - seen < 5 * 60_000
        ? "Active now"
        : `Last seen ${formatAgo(seen, nowMs)}`;
  return {
    id: device.id,
    name: device.name,
    kind: deviceKind(device.name),
    isThisDevice,
    lastSeen,
    enrolled: created == null ? "Enrollment date unknown" : `Enrolled ${formatShortDate(created, nowMs)}`,
    stale,
    duplicateName: (ctx.nameCounts.get(device.name) ?? 0) > 1,
  };
}

/** This device first, then most recently active; never-seen devices fall back to enrollment date. */
export function sortDevices(devices: readonly AnywhereDevice[], currentDeviceId: string | null | undefined): AnywhereDevice[] {
  const activity = (device: AnywhereDevice) => parse(device.last_seen_at) ?? parse(device.created_at) ?? 0;
  return [...devices].sort((a, b) => {
    if (a.id === currentDeviceId) return -1;
    if (b.id === currentDeviceId) return 1;
    return activity(b) - activity(a);
  });
}

export function describeDevices(
  devices: readonly AnywhereDevice[],
  currentDeviceId: string | null | undefined,
  nowMs: number,
): DeviceRowModel[] {
  const nameCounts = new Map<string, number>();
  for (const device of devices) nameCounts.set(device.name, (nameCounts.get(device.name) ?? 0) + 1);
  return sortDevices(devices, currentDeviceId).map((device) => describeDevice(device, { currentDeviceId, nowMs, nameCounts }));
}
