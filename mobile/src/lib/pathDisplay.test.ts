import { describe, expect, it } from "vitest";

import { commandProgram, friendlyCommand, shortenPath, tildify } from "./pathDisplay";

describe("path display", () => {
  it("folds the home folder wherever a path appears", () => {
    expect(tildify("/home/floris/.local/share/forge/x")).toBe("~/.local/share/forge/x");
    expect(tildify("cat /Users/ann/a.txt /root/b > /tmp/c")).toBe("cat ~/a.txt ~/b > /tmp/c");
    expect(tildify("--home=/home/floris")).toBe("--home=~");
    expect(tildify("/homestead/x /home")).toBe("/homestead/x /home");
  });

  it("middle-truncates a long path keeping its filename", () => {
    const path = "/home/floris/.local/share/forge/sessions/2026-10-26/transcripts/final-report.md";
    const short = shortenPath(path, 40);
    expect(short.length).toBeLessThanOrEqual(40);
    expect(short.endsWith("final-report.md")).toBe(true);
    expect(short).toContain("…");
    expect(shortenPath("/home/floris/a.rs", 40)).toBe("~/a.rs");
  });

  it("reads a command on one line without exec or the home folder", () => {
    expect(friendlyCommand("exec  /home/u/bin/run\n  --fast")).toBe("~/bin/run --fast");
    expect(commandProgram("exec /home/u/bin/agentlive-exec log -- 'x'")).toBe("agentlive-exec");
  });
});
