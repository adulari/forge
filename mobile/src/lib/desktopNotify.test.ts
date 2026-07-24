import { describe, expect, it } from "vitest";

import { decisionCopy, isContentLocked, redactTrayTitle } from "./desktopNotifyCore";

const notice = {
  sessionTitle: "Backtest vol-mom sweep",
  detail: "Permission: overwrite .env.staging",
};

describe("decisionCopy", () => {
  it("names the session and the request when the content is not locked", () => {
    expect(decisionCopy({ ...notice, locked: false })).toEqual({
      title: "Backtest vol-mom sweep needs you",
      body: "Permission: overwrite .env.staging",
    });
  });

  // The privacy guarantee, not a formatting preference: an OS notification for an
  // Anywhere-routed (or locked) session must not carry the title or the request.
  it("leaks neither the session title nor the request when locked", () => {
    const copy = decisionCopy({ ...notice, locked: true });
    const rendered = `${copy.title} ${copy.body}`;
    expect(rendered).not.toContain(notice.sessionTitle);
    expect(rendered).not.toContain(".env.staging");
    expect(copy.title).toBe("Forge needs you");
  });

  it("falls back to generic body copy when there is no detail to show", () => {
    expect(decisionCopy({ sessionTitle: "Fix mesh failover", locked: false }).body).toBe(
      "A decision is waiting.",
    );
  });
});

describe("tray + transport gates", () => {
  it("treats Anywhere transport as locked and direct/legacy rows as not", () => {
    expect(isContentLocked("anywhere")).toBe(true);
    expect(isContentLocked("direct")).toBe(false);
    expect(isContentLocked(undefined)).toBe(false);
  });

  it("redacts tray row titles under the same gate", () => {
    expect(redactTrayTitle("Backtest vol-mom sweep", true)).toBe("Session");
    expect(redactTrayTitle("Backtest vol-mom sweep", false)).toBe("Backtest vol-mom sweep");
  });
});
