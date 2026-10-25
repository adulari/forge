import { describe, expect, it } from "vitest";

import { accumulateStreaming, EMPTY_STREAMING, streamingDisplayText } from "./streamingText";

describe("accumulateStreaming", () => {
  it("is the tail itself for an older host or a reply that fits", () => {
    expect(accumulateStreaming(EMPTY_STREAMING, "hello", undefined).text).toBe("hello");
    expect(accumulateStreaming({ base: 0, text: "old reply" }, "new", 0).text).toBe("new");
  });

  it("keeps the start of a long reply once the wire only carries its tail", () => {
    let acc = accumulateStreaming(EMPTY_STREAMING, "abcdef", 0);
    acc = accumulateStreaming(acc, "defgh", 3);
    expect(acc).toEqual({ base: 0, text: "abcdefgh" });
    acc = accumulateStreaming(acc, "ghij", 6);
    expect(streamingDisplayText(acc)).toBe("abcdefghij");
  });

  it("marks a reply joined mid-way instead of pretending it starts there", () => {
    const acc = accumulateStreaming(EMPTY_STREAMING, "tail text", 5000);
    expect(acc.base).toBe(5000);
    expect(streamingDisplayText(acc)).toBe("…tail text");
    expect(streamingDisplayText(accumulateStreaming(acc, "il text more", 5002))).toBe("…tail text more");
  });

  it("keeps its identity when a frame changes nothing", () => {
    const acc = accumulateStreaming(EMPTY_STREAMING, "same", 0);
    expect(accumulateStreaming(acc, "same", 0)).toBe(acc);
  });
});
