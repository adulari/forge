import { describe, expect, it } from "vitest";

import { commandSummary } from "./commandSummary";

describe("commandSummary", () => {
  it("drops a leading cd and the bash -lc wrapper", () => {
    expect(commandSummary(`bash -lc 'cd /repo/app && cargo test -p forge'`)).toBe("cargo test -p forge");
    expect(commandSummary(`cd "/repo/my app" && npm run build`)).toBe("npm run build");
    expect(commandSummary(`/bin/bash -c "cd x; ls -la"`)).toBe("ls -la");
  });

  it("names a sed range as a view of those lines", () => {
    expect(commandSummary(`sed -n '10,40p' src/lib.rs`)).toBe("View src/lib.rs 10-40");
    expect(commandSummary(`bash -lc "cd /r && sed -n '7p' a.ts"`)).toBe("View a.ts 7");
    expect(commandSummary(`sed -n 5,5p a.ts`)).toBe("View a.ts 5");
  });

  it("reads a command whose closing quote was cut off by the stored cap", () => {
    expect(commandSummary(`bash -lc 'cd /repo && sed -n "10,40p" src/config.…`)).toBe("View src/config.… 10-40");
    expect(commandSummary(`bash -lc 'cd /repo && cargo test -p for…`)).toBe("cargo test -p for…");
  });

  it("leaves other commands alone, home folded and whitespace collapsed", () => {
    expect(commandSummary(`git   status   --short`)).toBe("git status --short");
    expect(commandSummary(`sed -i 's/a/b/' f`)).toBe("sed -i 's/a/b/' f");
    expect(commandSummary(`ls /home/ana/projects`)).toBe("ls ~/projects");
  });

  it("keeps the original when stripping would leave nothing", () => {
    expect(commandSummary("cd /tmp")).toBe("cd /tmp");
  });
});
