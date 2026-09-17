/// <reference types="vite/client" />

// Render-testing MessageRow needs the full RN/Reanimated tree; this repo's convention for such
// components (see Composer.nativeLayout.test.ts) is asserting on the raw source instead.
import { describe, expect, it } from "vitest";

import messageRowSource from "./MessageRow.tsx?raw";

const normalizedSource = messageRowSource.replace(/\s+/g, " ");

describe("MessageRow system-row classification", () => {
  it("renders a kind: \"system\" assistant row (the published answer) as prose", () => {
    // The daemon projects every ui-visibility row as kind "system", and the accepted answer of a
    // turn is published as one, so kind must not route a row to SystemOutput.
    expect(normalizedSource).toMatch(
      /const isSystem = row\.role === "system" \|\| row\.role === "tool" \|\| row\.kind === "tool";/,
    );
  });
});
