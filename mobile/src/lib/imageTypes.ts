export interface ViewerImage {
  uri: string;
  name: string;
  caption?: string | null;
}

const EXTENSION_BY_TYPE: Record<string, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/gif": "gif",
  "image/webp": "webp",
  "image/svg+xml": "svg",
};

/** A file name for a saved image: the display name, given an extension when it has none. */
export function downloadName(image: ViewerImage, mime?: string): string {
  const base = image.name.replace(/[\\/:*?"<>|\u0000-\u001f]/g, "_").trim() || "image";
  if (/\.(png|jpe?g|gif|webp|svg)$/i.test(base)) return base;
  return `${base}.${(mime && EXTENSION_BY_TYPE[mime]) || "png"}`;
}
