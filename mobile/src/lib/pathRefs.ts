// Repo paths the chat can make clickable: `src/a.ts:12`, `crates/x/lib.rs`, `~/notes.md`, a bare
// `Cargo.toml`. Detection is deliberately conservative — a path chip that was never a path is
// worse than a path left as plain text — so a bare name needs a known extension and a name that
// is not a product (`Node.js`), and a name with a folder only needs an extension that looks like one.

export interface PathRef {
  path: string;
  line: number | null;
  column: number | null;
}

export type PathPart = { type: "text"; text: string } | { type: "path"; text: string; ref: PathRef };

const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "gif", "webp", "svg"];

const KNOWN_EXTENSIONS = [
  ...IMAGE_EXTENSIONS,
  "ts", "tsx", "js", "jsx", "mjs", "cjs", "rs", "py", "go", "java", "kt", "kts", "swift", "c", "h", "cc",
  "cpp", "hpp", "cs", "rb", "php", "sh", "bash", "zsh", "fish", "ps1", "lua", "zig", "dart", "scala", "ex",
  "exs", "erl", "hs", "ml", "clj", "vue", "svelte", "astro", "json", "jsonc", "toml", "yaml", "yml", "xml",
  "ini", "env", "conf", "cfg", "lock", "gradle", "proto", "graphql", "gql", "sql", "tf", "nix", "md", "mdx",
  "txt", "rst", "html", "css", "scss", "sass", "less", "csv", "log", "patch", "diff", "dockerfile",
];

const PRODUCT_NAMES = new Set([
  "node.js", "next.js", "nuxt.js", "vue.js", "react.js", "express.js", "chart.js", "d3.js", "three.js",
  "ember.js", "angular.js", "backbone.js", "nest.js", "deno.js", "bun.js", "p5.js", "alpine.js", "solid.js",
]);

const EXTENSION_GROUP = [...KNOWN_EXTENSIONS].sort((a, b) => b.length - a.length).join("|");

// A path is a run of name characters joined by `/`, optionally rooted (`/`, `~/`, `./`, `../`),
// ending in a known extension, with an optional `:line[:col]`, `:line-line` or `#Lline` suffix.
// It may not start in the middle of a word, a URL or an email.
const PATH_SOURCE =
  String.raw`(?<![\w@:/.~\\-])(?!@)((?:~\/|\.{1,2}\/|\/)?(?:[\w.@+-]+\/)*[\w@+-][\w.@+-]*\.(?:${EXTENSION_GROUP}))(?::(\d{1,6})(?::(\d{1,5}))?(?:-\d{1,6})?|#L(\d{1,6}))?(?![\w/@-]|\.\w)`;

function pathRegex(): RegExp {
  return new RegExp(PATH_SOURCE, "gi");
}

function extensionOf(path: string): string {
  const name = path.slice(path.lastIndexOf("/") + 1);
  const dot = name.lastIndexOf(".");
  return dot === -1 ? "" : name.slice(dot + 1).toLowerCase();
}

export function isImagePath(path: string): boolean {
  return IMAGE_EXTENSIONS.includes(extensionOf(path));
}

function acceptable(path: string): boolean {
  if (path.includes("/")) return !/^\.{1,2}$|\.\.\.|\/\.\.?\/\.\./.test(path);
  return !PRODUCT_NAMES.has(path.toLowerCase());
}

function toRef(match: RegExpExecArray): PathRef | null {
  const path = match[1];
  if (!acceptable(path)) return null;
  const line = match[2] ?? match[4];
  return {
    path,
    line: line == null ? null : Number(line),
    column: match[3] == null ? null : Number(match[3]),
  };
}

/** `text` cut into plain runs and path references, in order. */
export function splitPathRefs(text: string): PathPart[] {
  const parts: PathPart[] = [];
  const re = pathRegex();
  let last = 0;
  let match: RegExpExecArray | null;
  while ((match = re.exec(text))) {
    const ref = toRef(match);
    if (!ref) continue;
    if (match.index > last) parts.push({ type: "text", text: text.slice(last, match.index) });
    parts.push({ type: "path", text: match[0], ref });
    last = match.index + match[0].length;
  }
  if (last < text.length) parts.push({ type: "text", text: text.slice(last) });
  return parts;
}

/** The reference an inline code span names, when the span is nothing but a path. */
export function pathRefOfCode(code: string): PathRef | null {
  const text = code.trim();
  const match = pathRegex().exec(text);
  if (!match || match[0].length !== text.length) return null;
  return toRef(match);
}

/** `@path` mentions in a prompt (`@src/a.ts`, `@{dir with space/a.ts}`), with their text spans. */
export function splitMentions(text: string): PathPart[] {
  const parts: PathPart[] = [];
  const re = /(^|\s)@(?:\{([^}\n]+)\}|([^\s@{}]+))/g;
  let last = 0;
  let match: RegExpExecArray | null;
  while ((match = re.exec(text))) {
    const raw = (match[2] ?? match[3]).replace(/[.,;:!?)]+$/, "");
    if (!raw.includes(".") && !raw.includes("/")) continue;
    const start = match.index + match[1].length;
    const end = start + 1 + (match[2] != null ? raw.length + 2 : raw.length);
    if (start > last) parts.push({ type: "text", text: text.slice(last, start) });
    parts.push({ type: "path", text: text.slice(start, end), ref: { path: raw, line: null, column: null } });
    last = end;
  }
  if (last < text.length) parts.push({ type: "text", text: text.slice(last) });
  return parts;
}

/** A path as the workspace API wants it: relative to the session's folder when it lies inside it. */
export function workspaceRelative(path: string, roots: readonly (string | null | undefined)[]): string {
  let clean = path.startsWith("file://") ? path.slice(7) : path;
  for (const root of roots) {
    if (!root) continue;
    const prefix = root.endsWith("/") ? root : `${root}/`;
    if (clean.startsWith(prefix)) return clean.slice(prefix.length);
  }
  if (clean.startsWith("./")) clean = clean.slice(2);
  return clean;
}

export type FileKind =
  | "typescript" | "javascript" | "rust" | "python" | "go" | "jvm" | "swift" | "c" | "ruby" | "shell"
  | "web" | "style" | "data" | "docs" | "image" | "other";

const KIND_BY_EXTENSION: Record<string, FileKind> = {
  ts: "typescript", tsx: "typescript", js: "javascript", jsx: "javascript", mjs: "javascript", cjs: "javascript",
  rs: "rust", py: "python", go: "go", java: "jvm", kt: "jvm", kts: "jvm", scala: "jvm", gradle: "jvm",
  swift: "swift", c: "c", h: "c", cc: "c", cpp: "c", hpp: "c", cs: "c", rb: "ruby", php: "ruby",
  sh: "shell", bash: "shell", zsh: "shell", fish: "shell", ps1: "shell",
  html: "web", vue: "web", svelte: "web", astro: "web", css: "style", scss: "style", sass: "style", less: "style",
  json: "data", jsonc: "data", toml: "data", yaml: "data", yml: "data", xml: "data", ini: "data", env: "data",
  conf: "data", cfg: "data", lock: "data", csv: "data", sql: "data", proto: "data", graphql: "data", gql: "data",
  md: "docs", mdx: "docs", txt: "docs", rst: "docs", log: "docs",
  png: "image", jpg: "image", jpeg: "image", gif: "image", webp: "image", svg: "image",
};

export function fileKind(path: string): FileKind {
  return KIND_BY_EXTENSION[extensionOf(path)] ?? "other";
}

/** Hue per kind; the chip turns it into a colour for the active theme. `other` has no hue. */
const KIND_HUE: Record<FileKind, number | null> = {
  typescript: 212, javascript: 48, rust: 22, python: 140, go: 188, jvm: 8, swift: 14, c: 262, ruby: 350,
  shell: 120, web: 28, style: 290, data: 168, docs: 220, image: 320, other: null,
};

export function kindHue(kind: FileKind): number | null {
  return KIND_HUE[kind];
}

export function baseName(path: string): string {
  return path.slice(path.lastIndexOf("/") + 1) || path;
}

/** `name` or `name:12`, what a chip shows. */
export function chipLabel(ref: PathRef): string {
  return ref.line == null ? baseName(ref.path) : `${baseName(ref.path)}:${ref.line}`;
}
