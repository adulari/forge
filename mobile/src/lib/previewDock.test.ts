import { beforeEach, describe, expect, it, vi } from "vitest";

import { __consoleTesting, appendConsoleEntries, CONSOLE_LIMIT, consoleCounts, getConsoleEntries } from "./previewConsole";
import { isLoopbackUrl, parseRecents, pushRecent, recentLabel } from "./previewRecents";
import { pickTerminalId } from "./terminalInsert";

vi.mock("@react-native-async-storage/async-storage", () => ({
  default: { getItem: vi.fn(), setItem: vi.fn(), removeItem: vi.fn() },
}));
vi.mock("./api", () => ({ listTerminalSessions: vi.fn(), openTerminalSocket: vi.fn() }));

beforeEach(() => __consoleTesting.reset());

describe("recent preview URLs", () => {
  it("moves a revisited address to the front without duplicating it", () => {
    const list = ["http://localhost:3000/", "http://localhost:5173/"];
    expect(pushRecent(list, "http://localhost:5173")).toEqual(["http://localhost:5173", "http://localhost:3000/"]);
  });

  it("caps the list and survives bad storage", () => {
    const many = Array.from({ length: 12 }, (_, index) => `http://localhost:${3000 + index}/`);
    expect(pushRecent(many, "http://localhost:9/")).toHaveLength(8);
    expect(parseRecents("not json")).toEqual([]);
    expect(parseRecents('{"a":1}')).toEqual([]);
    expect(parseRecents('["http://a/", 4, null]')).toEqual(["http://a/"]);
  });

  it("labels rows and tells loopback from the open web", () => {
    expect(recentLabel("http://localhost:5173/app?x=1")).toBe("localhost:5173/app?x=1");
    expect(recentLabel("http://localhost:5173/")).toBe("localhost:5173");
    expect(isLoopbackUrl("http://localhost:5173/")).toBe(true);
    expect(isLoopbackUrl("http://127.0.0.1:8080/x")).toBe(true);
    expect(isLoopbackUrl("http://[::1]:3000/")).toBe(true);
    expect(isLoopbackUrl("https://example.com/")).toBe(false);
    expect(isLoopbackUrl("http://localhost.evil.com/")).toBe(false);
    expect(isLoopbackUrl("nonsense")).toBe(false);
  });
});

describe("preview console", () => {
  it("keeps the newest lines and counts problems", () => {
    appendConsoleEntries("p", [{ level: "error", text: "boom" }]);
    appendConsoleEntries(
      "p",
      Array.from({ length: CONSOLE_LIMIT }, (_, index) => ({ level: "warn" as const, text: `w${index}` })),
    );
    const entries = getConsoleEntries("p");
    expect(entries).toHaveLength(CONSOLE_LIMIT);
    expect(entries[0]?.text).toBe("w0");
    expect(consoleCounts(entries)).toEqual({ error: 0, warn: CONSOLE_LIMIT });
  });

  it("keeps previews apart", () => {
    appendConsoleEntries("a", [{ level: "log", text: "one" }]);
    expect(getConsoleEntries("b")).toEqual([]);
  });
});

describe("insert into terminal", () => {
  it("targets the first running terminal", () => {
    expect(
      pickTerminalId([
        { terminal_id: "term-1", status: "exited" },
        { terminal_id: "term-2", status: "running" },
      ]),
    ).toBe("term-2");
    expect(pickTerminalId([{ terminal_id: "term-1", status: "exited" }])).toBeNull();
  });
});
