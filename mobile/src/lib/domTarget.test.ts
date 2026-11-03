import { describe, expect, it } from "vitest";

import { closestElement, containsNode } from "./domTarget";

// Mirrors WebKit: Node.contains throws a TypeError for anything that is not a Node.
function strictRoot(inside: object[]): Node {
  return {
    nodeType: 1,
    contains(other: unknown) {
      if (typeof other !== "object" || other === null || typeof (other as { nodeType?: unknown }).nodeType !== "number") {
        throw new TypeError("Argument 1 ('other') to Node.contains must be an instance of Node");
      }
      return inside.includes(other as object);
    },
  } as unknown as Node;
}

describe("containsNode", () => {
  const child = { nodeType: 1 };
  const root = strictRoot([child]);

  it("answers for real nodes", () => {
    expect(containsNode(root, child)).toBe(true);
    expect(containsNode(root, { nodeType: 1 })).toBe(false);
  });

  it("is false, not a throw, for window, strings, null and undefined", () => {
    const fakeWindow = { window: {}, document: {} };
    for (const target of [fakeWindow, "body", null, undefined, 42]) {
      expect(() => root.contains(target as never)).toThrow(TypeError);
      expect(containsNode(root, target)).toBe(false);
    }
  });

  it("is false for a missing root", () => {
    expect(containsNode(null, { nodeType: 1 })).toBe(false);
    expect(containsNode(undefined, { nodeType: 1 })).toBe(false);
  });
});

describe("closestElement", () => {
  it("delegates to closest and tolerates targets without it", () => {
    const hit = { nodeType: 1 };
    expect(closestElement({ closest: () => hit }, "x")).toBe(hit);
    expect(closestElement({ window: {} }, "x")).toBeNull();
    expect(closestElement(null, "x")).toBeNull();
  });
});
