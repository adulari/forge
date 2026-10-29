import { describe, expect, it } from "vitest";

import {
  fileKind,
  isImagePath,
  kindHue,
  pathRefOfCode,
  splitMentions,
  splitPathRefs,
  workspaceRelative,
} from "./pathRefs";

const paths = (text: string) => splitPathRefs(text).flatMap((p) => (p.type === "path" ? [p.ref] : []));

describe("splitPathRefs", () => {
  it("finds a path with a line and a column in prose", () => {
    expect(paths("The bug is in src/a.ts:12 and crates/x/lib.rs:7:3.")).toEqual([
      { path: "src/a.ts", line: 12, column: null },
      { path: "crates/x/lib.rs", line: 7, column: 3 },
    ]);
  });

  it("keeps the text around a path intact", () => {
    const parts = splitPathRefs("see `src/a.ts` now");
    expect(parts.map((p) => p.text).join("")).toBe("see `src/a.ts` now");
  });

  it("takes bare filenames with a known extension but not product names", () => {
    expect(paths("edit Cargo.toml and README.md")).toHaveLength(2);
    expect(paths("built with Node.js and Next.js")).toEqual([]);
  });

  it("reads GitHub-style #L anchors and line ranges", () => {
    expect(paths("src/b.py#L40")[0]).toEqual({ path: "src/b.py", line: 40, column: null });
    expect(paths("src/b.py:10-20")[0].line).toBe(10);
  });

  it("leaves URLs, emails, versions and sentence punctuation alone", () => {
    expect(paths("https://example.com/docs/guide.md")).toEqual([]);
    expect(paths("mail me@host.com or see v1.2.3 and e.g. this")).toEqual([]);
    expect(paths("Open src/a.ts.")[0].path).toBe("src/a.ts");
    expect(paths("(see docs/x.md)")[0].path).toBe("docs/x.md");
  });

  it("accepts rooted paths", () => {
    expect(paths("run ./scripts/dev.sh then ~/notes/todo.md")).toHaveLength(2);
    expect(paths("/home/u/proj/src/a.ts:3")[0].path).toBe("/home/u/proj/src/a.ts");
  });

  it("does not match inside a longer word or path segment", () => {
    expect(paths("foo/bar.tsx2")).toEqual([]);
    expect(paths("a/b.ts/c")).toEqual([]);
  });
});

describe("pathRefOfCode", () => {
  it("accepts a span that is only a path", () => {
    expect(pathRefOfCode("src/a.ts:12")).toEqual({ path: "src/a.ts", line: 12, column: null });
    expect(pathRefOfCode(" docs/img/shot.png ")?.path).toBe("docs/img/shot.png");
  });

  it("rejects code that merely contains one", () => {
    expect(pathRefOfCode("cat src/a.ts")).toBeNull();
    expect(pathRefOfCode("npm install")).toBeNull();
  });
});

describe("splitMentions", () => {
  it("finds @path and @{path with spaces}", () => {
    const refs = splitMentions("fix @src/a.ts and @{my dir/b.ts} please")
      .flatMap((p) => (p.type === "path" ? [p.ref.path] : []));
    expect(refs).toEqual(["src/a.ts", "my dir/b.ts"]);
  });

  it("ignores handles and mid-word @", () => {
    expect(splitMentions("ping @alice, mail a@b.co")).toEqual([{ type: "text", text: "ping @alice, mail a@b.co" }]);
  });
});

describe("workspaceRelative", () => {
  it("strips a session root and a leading ./", () => {
    expect(workspaceRelative("/w/p/src/a.ts", ["/w/p"])).toBe("src/a.ts");
    expect(workspaceRelative("./src/a.ts", [])).toBe("src/a.ts");
    expect(workspaceRelative("/elsewhere/a.ts", ["/w/p"])).toBe("/elsewhere/a.ts");
  });
});

describe("file kinds", () => {
  it("classifies by extension", () => {
    expect(fileKind("a/b.tsx")).toBe("typescript");
    expect(fileKind("Cargo.toml")).toBe("data");
    expect(fileKind("x.unknown")).toBe("other");
    expect(kindHue("other")).toBeNull();
    expect(isImagePath("shot.PNG")).toBe(true);
    expect(isImagePath("a.ts")).toBe(false);
  });
});
