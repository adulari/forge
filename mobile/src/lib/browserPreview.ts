import { isTauri } from "./platform";

export const PREVIEW_STATE_EVENT = "forge://preview-state";
export const PREVIEW_ANNOTATION_EVENT = "forge://preview-annotation";

export type PreviewViewport = "fill" | "mobile" | "tablet";

export interface PreviewBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface PreviewStateEvent {
  label: string;
  url: string;
  loaded: boolean;
}

export interface PreviewElementAttribute {
  name: string;
  value: string;
}

export interface PreviewElementAnnotation {
  url: string;
  title: string;
  selector: string;
  tagName: string;
  elementId: string | null;
  role: string | null;
  accessibleName: string | null;
  text: string;
  attributes: PreviewElementAttribute[];
  rect: { x: number; y: number; width: number; height: number };
}

export interface PreviewAnnotationEvent {
  label: string;
  annotation: PreviewElementAnnotation;
}

export interface PreviewRuntimePreferences {
  url: string;
  zoom: number;
  viewport: PreviewViewport;
}

const DEFAULT_PREFERENCES: PreviewRuntimePreferences = {
  url: "",
  zoom: 1,
  viewport: "fill",
};
const preferencesByLabel = new Map<string, PreviewRuntimePreferences>();

function hashPart(value: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return (hash >>> 0).toString(16).padStart(8, "0");
}

/** Tauri labels permit a narrow character set. Two directional hashes keep long resource IDs out. */
export function previewLabel(surfaceId: string): string {
  return `preview-${hashPart(surfaceId)}-${hashPart([...surfaceId].reverse().join(""))}`;
}

export function getPreviewPreferences(label: string): PreviewRuntimePreferences {
  return preferencesByLabel.get(label) ?? DEFAULT_PREFERENCES;
}

export function setPreviewPreferences(
  label: string,
  patch: Partial<PreviewRuntimePreferences>,
): PreviewRuntimePreferences {
  const next = { ...getPreviewPreferences(label), ...patch };
  preferencesByLabel.set(label, next);
  return next;
}

function defaultScheme(value: string): string {
  const barePort = /^\d{1,5}(?:\/|$)/.test(value);
  const local =
    /^(localhost|127(?:\.\d{1,3}){3}|\[?::1\]?)(?::\d+)?(?:\/|$)/i.test(value) ||
    barePort;
  return `${local ? "http" : "https"}://${barePort ? `localhost:${value}` : value}`;
}

/** Accept a URL, hostname, localhost address, or bare port; reject every non-HTTP scheme. */
export function normalizePreviewUrl(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed) throw new Error("Enter a URL or local port.");
  const candidate =
    /^[a-z][a-z\d+.-]*:\/\//i.test(trimmed) ||
    /^(?:blob|data|file|ftp|javascript|mailto|ws|wss):/i.test(trimmed)
      ? trimmed
      : defaultScheme(trimmed);
  let url: URL;
  try {
    url = new URL(candidate);
  } catch {
    throw new Error("Enter a valid URL or local port.");
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new Error("Preview URLs must use HTTP or HTTPS.");
  }
  if (!url.hostname) throw new Error("Preview URL is missing a host.");
  return url.toString();
}

export function previewViewportBounds(
  measured: PreviewBounds,
  viewport: PreviewViewport,
): PreviewBounds {
  const requestedWidth = viewport === "mobile" ? 390 : viewport === "tablet" ? 768 : measured.width;
  const width = Math.max(1, Math.min(measured.width, requestedWidth));
  return {
    x: measured.x + (measured.width - width) / 2,
    y: measured.y,
    width,
    height: Math.max(1, measured.height),
  };
}

async function invoke<T>(command: string, args: Record<string, unknown>): Promise<T> {
  if (!isTauri) throw new Error("Browser preview is available in the Forge desktop app.");
  const core = await import("@tauri-apps/api/core");
  return core.invoke<T>(command, args);
}

export function openBrowserPreview(
  label: string,
  url: string,
  bounds: PreviewBounds,
): Promise<PreviewStateEvent> {
  return invoke("preview_open", { label, url, bounds });
}

export function setBrowserPreviewBounds(label: string, bounds: PreviewBounds): Promise<void> {
  return invoke("preview_set_bounds", { label, bounds });
}

export function navigateBrowserPreview(label: string, url: string): Promise<void> {
  return invoke("preview_navigate", { label, url });
}

export function moveBrowserPreviewHistory(label: string, direction: -1 | 1): Promise<void> {
  return invoke("preview_history", { label, direction });
}

export function reloadBrowserPreview(label: string): Promise<void> {
  return invoke("preview_reload", { label });
}

export function zoomBrowserPreview(label: string, zoom: number): Promise<void> {
  return invoke("preview_set_zoom", { label, zoom });
}

export function setBrowserPreviewPicker(label: string, active: boolean): Promise<void> {
  return invoke("preview_set_picker", { label, active });
}

export function hideBrowserPreview(label: string): Promise<void> {
  return invoke("preview_hide", { label });
}

export async function listenBrowserPreview(
  onState: (event: PreviewStateEvent) => void,
  onAnnotation: (event: PreviewAnnotationEvent) => void,
): Promise<() => void> {
  if (!isTauri) return () => undefined;
  const { listen } = await import("@tauri-apps/api/event");
  const [unlistenState, unlistenAnnotation] = await Promise.all([
    listen<PreviewStateEvent>(PREVIEW_STATE_EVENT, (event) => onState(event.payload)),
    listen<PreviewAnnotationEvent>(PREVIEW_ANNOTATION_EVENT, (event) =>
      onAnnotation(event.payload),
    ),
  ]);
  return () => {
    unlistenState();
    unlistenAnnotation();
  };
}

export const __testing = {
  clearPreferences(): void {
    preferencesByLabel.clear();
  },
};
