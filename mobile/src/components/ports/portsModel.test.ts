import { describe, expect, it } from "vitest";

import { parsePorts, parseProjectActions, parseRunningActions, runningSessionFor, type ProjectAction } from "../../lib/projectActionsModel";
import {
  actionIdOfTerminal,
  actionPaletteEntries,
  actionStatus,
  actionTerminalNames,
  daemonIsLocal,
  exitBadge,
  hostBadge,
  portLink,
  runningBySession,
  runningLabel,
  sourceLabel,
} from "./portsModel";

const action = (patch: Partial<ProjectAction> = {}): ProjectAction => ({
  id: "dev",
  name: "Dev server",
  command: "npm run dev",
  cwd: null,
  icon: null,
  terminal_id: null,
  running: false,
  exit_code: null,
  ...patch,
});

describe("parsing", () => {
  it("fills defaults and drops rows without an id", () => {
    const parsed = parseProjectActions({ actions: [{ id: "dev", command: "x" }, { name: "no id" }, null], untrusted: false });
    expect(parsed.actions).toEqual([action({ name: "dev", command: "x" })]);
    expect(parsed.untrusted).toBe(false);
  });

  it("survives a malformed body", () => {
    expect(parseProjectActions(null)).toEqual({ actions: [], untrusted: false });
    expect(parseProjectActions({ untrusted: true }).untrusted).toBe(true);
    expect(parseRunningActions({ running: "x" })).toEqual([]);
    expect(parsePorts(undefined)).toEqual([]);
  });

  it("sorts ports, rejects bad ones and defaults unknown hosts to loopback", () => {
    const ports = parsePorts({
      ports: [
        { port: 8080, host: "all", pid: 2, process: "node", command: "node s.js", source: "terminal" },
        { port: 0 },
        { port: 70000 },
        { port: 5173, host: "weird", pid: 1, process: "python3", command: "python3 -m http.server", source: "action", action: "dev" },
      ],
    });
    expect(ports.map((p) => p.port)).toEqual([5173, 8080]);
    expect(ports[0].host).toBe("loopback");
    expect(ports[0].action).toBe("dev");
  });

  it("keeps only complete running rows", () => {
    expect(parseRunningActions({ running: [{ session: "s", action: "dev" }, { session: "s" }] })).toEqual([{ session: "s", action: "dev" }]);
  });
});

describe("action status", () => {
  it("distinguishes running, failed, exited and idle", () => {
    expect(actionStatus(action({ running: true, terminal_id: "act-dev" }))).toBe("running");
    expect(actionStatus(action({ terminal_id: "act-dev", exit_code: 2 }))).toBe("failed");
    expect(actionStatus(action({ terminal_id: "act-dev", exit_code: 0 }))).toBe("exited");
    expect(actionStatus(action())).toBe("idle");
  });

  it("badges only non-zero exits of stopped actions", () => {
    expect(exitBadge(action({ exit_code: 1 }))).toBe("exit 1");
    expect(exitBadge(action({ exit_code: 0 }))).toBeNull();
    expect(exitBadge(action({ exit_code: 1, running: true }))).toBeNull();
  });
});

describe("terminal ids", () => {
  it("maps terminals to actions and back to names", () => {
    expect(actionIdOfTerminal("act-dev")).toBe("dev");
    expect(actionIdOfTerminal("term-1")).toBeNull();
    expect(actionTerminalNames([action()]).get("act-dev")).toBe("Dev server");
  });
});

describe("ports", () => {
  it("labels hosts and sources", () => {
    expect(hostBadge("loopback").label).toBe("localhost");
    expect(hostBadge("all").label).toBe("network");
    const port = { port: 1, host: "all", pid: 1, process: "p", command: "c", source: "action", action: "dev" } as const;
    expect(sourceLabel(port, [action()])).toBe("Dev server");
    expect(sourceLabel({ ...port, action: "gone" }, [action()])).toBe("gone");
    expect(sourceLabel({ ...port, source: "workspace" }, [])).toBe("workspace");
  });

  it("links to localhost for a local daemon", () => {
    expect(portLink({ port: 5173, host: "loopback" }, "http://127.0.0.1:7420")).toEqual({ url: "http://localhost:5173", note: null });
    expect(daemonIsLocal(null)).toBe(true);
  });

  it("uses the daemon host remotely and warns for loopback-only ports", () => {
    const loop = portLink({ port: 5173, host: "loopback" }, "http://devbox.lan:7420");
    expect(loop.url).toBe("http://devbox.lan:5173");
    expect(loop.note).toMatch(/loopback/);
    expect(portLink({ port: 5173, host: "all" }, "https://10.0.0.5:7420")).toEqual({ url: "http://10.0.0.5:5173", note: null });
    expect(portLink({ port: 1, host: "all" }, "http://[fd00::1]:7420").url).toBe("http://[fd00::1]:1");
  });
});

describe("running indicator", () => {
  it("groups by session and words the label", () => {
    const grouped = runningBySession([
      { session: "a", action: "dev" },
      { session: "a", action: "api" },
      { session: "b", action: "dev" },
    ]);
    expect(grouped.get("a")).toEqual(["dev", "api"]);
    expect(runningLabel(["dev"])).toBe("dev running");
    expect(runningLabel(["dev", "api"], new Map([["dev", "Dev"]]))).toBe("Dev, api running");
    expect(runningLabel(["a", "b", "c"])).toBe("3 actions running");
    expect(runningLabel([])).toBe("");
  });
});

describe("palette entries", () => {
  it("offers Run when stopped and Stop when running", () => {
    const entries = actionPaletteEntries([action(), action({ id: "api", name: "api", running: true })]);
    expect(entries.map((e) => e.title)).toEqual(["Run: Dev server", "Stop: api"]);
    expect(entries.map((e) => e.kind)).toEqual(["run", "stop"]);
  });
});

describe("runningSessionFor", () => {
  const running = [
    { session: "a", action: "dev" },
    { session: "z", action: "dev" },
  ];
  it("finds the sibling session that runs the action", () => {
    expect(runningSessionFor(running, ["b", "a"], "dev")).toBe("a");
  });
  it("ignores sessions outside the project and other actions", () => {
    expect(runningSessionFor(running, ["b", "c"], "dev")).toBeNull();
    expect(runningSessionFor(running, ["a"], "lint")).toBeNull();
  });
});
