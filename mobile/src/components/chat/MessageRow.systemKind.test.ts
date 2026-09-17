/// <reference types="vite/client" />

// Render-testing MessageRow needs the full RN/Reanimated tree; this repo's convention for such
// components (see Composer.nativeLayout.test.ts) is asserting on the raw source instead.
import { describe, expect, it } from "vitest";

import messageRowSource from "./MessageRow.tsx?raw";

const normalizedSource = messageRowSource.replace(/\s+/g, " ");

describe("MessageRow system-row classification", () => {
  it("treats kind: \"system\" as a system row even when role is \"assistant\"", () => {
    // The daemon's completion/status notes carry `role: "assistant"` + `kind: "system"` on the
    // wire (remote_projection.rs) — matching only `role` here routed them through the ordinary
    // chat-bubble path instead of SystemOutput.
    expect(normalizedSource).toMatch(
      /const isSystem = row\.role === "system" \|\| row\.role === "tool" \|\| row\.kind === "tool" \|\| row\.kind === "system";/,
    );
  });
});
