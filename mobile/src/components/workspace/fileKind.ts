export type FileViewKind = "svg" | "pdf" | "markdown" | "text";

function extensionOf(path: string): string {
  const name = path.split("/").filter(Boolean).pop() ?? "";
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
}

export function fileViewKind(path: string): FileViewKind {
  switch (extensionOf(path)) {
    case "svg":
      return "svg";
    case "pdf":
      return "pdf";
    case "md":
    case "markdown":
    case "mdx":
      return "markdown";
    default:
      return "text";
  }
}

/** The name `highlightTokens` resolves; unknown extensions come back as-is and render plain. */
export function highlightLanguage(path: string): string {
  return extensionOf(path);
}

export type FileViewMode = "edit" | "code" | "preview";

export function hasRenderedPreview(kind: FileViewKind, canRenderSvg: boolean): boolean {
  return kind === "markdown" || (kind === "svg" && canRenderSvg);
}

/** A file opened at a line is for editing; a rendered kind opens rendered; the rest edit. */
export function defaultViewMode(kind: FileViewKind, line: number | null, canRenderSvg: boolean): FileViewMode {
  if (line != null) return "edit";
  return hasRenderedPreview(kind, canRenderSvg) ? "preview" : "edit";
}
