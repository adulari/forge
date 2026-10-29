// Language-coloured file marks for the git dock. A two-to-three letter mono tag on a tinted tile
// reads at 18px where a generic document glyph does not, and the hue says what kind of file a row
// is before the path is read. Hues are mid-tone so the same value holds up on the light and dark
// surfaces (the tile fill is the hue at low alpha; the tag is the hue itself).

export interface FileMark {
  tag: string;
  /** Hex colour of the tag; the tile fill is derived from it at render time. */
  hue: string;
}

const NEUTRAL = "#8A8F98";

const BY_EXTENSION: Record<string, FileMark> = {
  rs: { tag: "RS", hue: "#D9733B" },
  ts: { tag: "TS", hue: "#3F86C9" },
  tsx: { tag: "TSX", hue: "#3F86C9" },
  js: { tag: "JS", hue: "#C2A21E" },
  jsx: { tag: "JSX", hue: "#C2A21E" },
  mjs: { tag: "JS", hue: "#C2A21E" },
  cjs: { tag: "JS", hue: "#C2A21E" },
  py: { tag: "PY", hue: "#4E8DBD" },
  go: { tag: "GO", hue: "#22A6C4" },
  java: { tag: "JV", hue: "#C46A4A" },
  kt: { tag: "KT", hue: "#8F6BD1" },
  swift: { tag: "SW", hue: "#E0643C" },
  c: { tag: "C", hue: "#5B8DB8" },
  h: { tag: "H", hue: "#5B8DB8" },
  cpp: { tag: "C++", hue: "#5B8DB8" },
  json: { tag: "JSON", hue: "#B8A53A" },
  toml: { tag: "TOML", hue: "#A66F4F" },
  yaml: { tag: "YML", hue: "#C46464" },
  yml: { tag: "YML", hue: "#C46464" },
  md: { tag: "MD", hue: "#6C93B8" },
  mdx: { tag: "MDX", hue: "#6C93B8" },
  sh: { tag: "SH", hue: "#4CA66B" },
  bash: { tag: "SH", hue: "#4CA66B" },
  zsh: { tag: "SH", hue: "#4CA66B" },
  css: { tag: "CSS", hue: "#7B5FC7" },
  scss: { tag: "CSS", hue: "#7B5FC7" },
  html: { tag: "HTML", hue: "#E0643C" },
  sql: { tag: "SQL", hue: "#C9774A" },
  lock: { tag: "LOCK", hue: NEUTRAL },
  png: { tag: "IMG", hue: "#B05FA6" },
  jpg: { tag: "IMG", hue: "#B05FA6" },
  jpeg: { tag: "IMG", hue: "#B05FA6" },
  gif: { tag: "IMG", hue: "#B05FA6" },
  svg: { tag: "SVG", hue: "#B05FA6" },
  webp: { tag: "IMG", hue: "#B05FA6" },
};

const BY_NAME: Record<string, FileMark> = {
  dockerfile: { tag: "DOCK", hue: "#3F86C9" },
  makefile: { tag: "MAKE", hue: "#4CA66B" },
  "cargo.lock": { tag: "LOCK", hue: NEUTRAL },
  "package-lock.json": { tag: "LOCK", hue: NEUTRAL },
  ".gitignore": { tag: "GIT", hue: "#D9733B" },
};

export function fileMark(path: string): FileMark {
  const name = path.slice(path.lastIndexOf("/") + 1).toLowerCase();
  const byName = BY_NAME[name];
  if (byName) return byName;
  const dot = name.lastIndexOf(".");
  if (dot > 0) {
    const known = BY_EXTENSION[name.slice(dot + 1)];
    if (known) return known;
    return { tag: name.slice(dot + 1, dot + 4).toUpperCase(), hue: NEUTRAL };
  }
  return { tag: "·", hue: NEUTRAL };
}
