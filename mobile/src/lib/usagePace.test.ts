import { describe, expect, it } from "vitest";

import { paceAnnotation } from "./usagePace";
import type { DisplayQuota } from "./usageWindows";

const HOUR = 3600;
const quota = (patch: Partial<DisplayQuota>): DisplayQuota => ({
  windowKind: "five_hour",
  status: "ok",
  fraction: 0.4,
  resetsAt: 1_000_000 + 4 * HOUR,
  seen: true,
  ...patch,
});

describe("paceAnnotation", () => {
  it("projects the burn rate instead of echoing the current fraction", () => {
    const pace = paceAnnotation(quota({ fraction: 0.4, status: "warning" }), 1_000_000);
    expect(pace).toEqual({ text: "runs out 1h 30m", tone: "danger" });
  });

  it("shows the projected percentage when the window will not run dry", () => {
    const pace = paceAnnotation(quota({ fraction: 0.5, status: "warning", resetsAt: 1_000_000 + 2.5 * HOUR }), 1_000_000);
    expect(pace).toEqual({ text: "≈100% by reset", tone: "warn" });
    const slower = paceAnnotation(quota({ fraction: 0.3, status: "warning", resetsAt: 1_000_000 + 2.5 * HOUR }), 1_000_000);
    expect(slower?.text).toBe("≈60% by reset");
  });

  it("flags an unflagged quota only when it would be empty before the reset", () => {
    expect(paceAnnotation(quota({ fraction: 0.4 }), 1_000_000)?.tone).toBe("danger");
    expect(paceAnnotation(quota({ fraction: 0.1 }), 1_000_000)).toBeNull();
  });

  it("ignores the first sliver of a window", () => {
    const early = quota({ fraction: 0.02, resetsAt: 1_000_000 + 5 * HOUR - 60 });
    expect(paceAnnotation(early, 1_000_000)).toBeNull();
  });

  it("has nothing to say without a reset time, a known window or a rate", () => {
    expect(paceAnnotation(quota({ resetsAt: null }), 0)).toBeNull();
    expect(paceAnnotation(quota({ windowKind: "secondary" }), 1_000_000)).toBeNull();
    expect(paceAnnotation(quota({ fraction: 0 }), 1_000_000)).toBeNull();
    expect(paceAnnotation(quota({ resetsAt: 1_000_000 - 1 }), 1_000_000)).toBeNull();
  });
});
