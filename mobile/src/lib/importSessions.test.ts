import { describe, expect, it } from "vitest";

import {
  type ImportCandidate,
  allSelected,
  IMPORT_BATCH,
  chunk,
  mergeReplies,
  conversationAge,
  filterCandidates,
  folderOptions,
  importButtonLabel,
  importToast,
  itemKey,
  pruneSelection,
  shortFolder,
  toggleAll,
} from "./importSessions";

const row = (over: Partial<ImportCandidate>): ImportCandidate => ({
  agent: "claude",
  id: "a",
  title: "fix the flaky test",
  cwd: "/home/me/api",
  cwd_exists: true,
  modified: 0,
  bytes: 10,
  imported_session_id: null,
  ...over,
});

const items = [
  row({ id: "c1" }),
  row({ id: "c2", title: "port the parser", cwd: "/home/me/cli" }),
  row({ agent: "codex", id: "x1", title: "list the files", cwd: "/home/me/api" }),
  row({ agent: "codex", id: "x2", title: "orphan", cwd: null, cwd_exists: false }),
  row({ id: "c3", title: "done already", imported_session_id: "s9" }),
];

describe("filterCandidates", () => {
  it("narrows by agent, folder and search text together", () => {
    expect(filterCandidates(items, { agent: "all", folder: null, query: "" })).toHaveLength(5);
    expect(filterCandidates(items, { agent: "codex", folder: null, query: "" }).map((c) => c.id)).toEqual(["x1", "x2"]);
    expect(filterCandidates(items, { agent: "all", folder: "/home/me/api", query: "" }).map((c) => c.id)).toEqual([
      "c1",
      "x1",
      "c3",
    ]);
    expect(filterCandidates(items, { agent: "all", folder: null, query: " PARSER " }).map((c) => c.id)).toEqual(["c2"]);
    expect(filterCandidates(items, { agent: "all", folder: null, query: "codex" }).map((c) => c.id)).toEqual(["x1", "x2"]);
  });

  it("treats a conversation without a folder as its own folder", () => {
    expect(filterCandidates(items, { agent: "all", folder: "(no folder)", query: "" }).map((c) => c.id)).toEqual(["x2"]);
  });
});

describe("folderOptions", () => {
  it("counts conversations per folder, busiest first", () => {
    expect(folderOptions(items)).toEqual([
      { folder: "/home/me/api", count: 3 },
      { folder: "(no folder)", count: 1 },
      { folder: "/home/me/cli", count: 1 },
    ]);
  });
});

describe("selection", () => {
  it("select-all skips what is already imported and toggles back off", () => {
    const all = toggleAll(items, new Set());
    expect([...all].sort()).toEqual(["claude:c1", "claude:c2", "codex:x1", "codex:x2"]);
    expect(allSelected(items, all)).toBe(true);
    expect(toggleAll(items, all).size).toBe(0);
  });

  it("select-all only touches the visible rows", () => {
    const codex = filterCandidates(items, { agent: "codex", folder: null, query: "" });
    const picked = toggleAll(codex, new Set([itemKey(items[0])]));
    expect([...picked].sort()).toEqual(["claude:c1", "codex:x1", "codex:x2"]);
  });

  it("is never all-selected when nothing can be selected", () => {
    expect(allSelected([items[4]], new Set())).toBe(false);
  });

  it("drops picks a rescan no longer offers", () => {
    const kept = pruneSelection(items, new Set(["claude:c1", "claude:c3", "codex:gone"]));
    expect([...kept]).toEqual(["claude:c1"]);
  });
});

describe("messages", () => {
  it("labels the import button and the toast with the right plural", () => {
    expect(importButtonLabel(0)).toBe("Import");
    expect(importButtonLabel(1)).toBe("Import 1 session");
    expect(importButtonLabel(2)).toBe("Import 2 sessions");
    expect(importToast({ imported: 1, skipped: 0, failed: 0 })).toEqual({ message: "Imported 1 session", tone: "success" });
    expect(importToast({ imported: 3, skipped: 1, failed: 0 }).message).toBe("Imported 3 sessions, 1 already in Forge");
    expect(importToast({ imported: 2, skipped: 0, failed: 1 })).toEqual({
      message: "Imported 2 sessions, 1 failed",
      tone: "warn",
    });
    expect(importToast({ imported: 0, skipped: 0, failed: 2 })).toEqual({
      message: "Nothing new imported, 2 failed",
      tone: "danger",
    });
  });

  it("shortens home folders and ages conversations", () => {
    expect(shortFolder("/home/me/api")).toBe("~/api");
    expect(shortFolder("/Users/me")).toBe("~");
    expect(shortFolder("/srv/app")).toBe("/srv/app");
    expect(shortFolder(null)).toBe("(no folder)");
    expect(conversationAge(100, 130_000)).toBe("just now");
    expect(conversationAge(0, 3 * 3600 * 1000)).toBe("3h ago");
    expect(conversationAge(0, 2 * 86_400 * 1000)).toBe("2d ago");
  });
});

describe("batching", () => {
  it("splits 1200 selected conversations into requests under the host cap and merges the replies", () => {
    const ids = Array.from({ length: 1200 }, (_, i) => i);
    const batches = chunk(ids, IMPORT_BATCH);
    expect(batches.every((b) => b.length <= IMPORT_BATCH && b.length <= 500)).toBe(true);
    expect(batches.flat()).toEqual(ids);
    expect(chunk([], IMPORT_BATCH)).toEqual([]);
    const merged = mergeReplies([
      { results: [], imported: 50, skipped: 1, failed: 0 },
      { results: [], imported: 49, skipped: 0, failed: 2 },
    ]);
    expect([merged.imported, merged.skipped, merged.failed]).toEqual([99, 1, 2]);
  });
});
