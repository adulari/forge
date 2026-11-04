import { beforeEach, describe, expect, it } from "vitest";

import { registerAgentSender, resetAgentBridgeForTests, sendToAgent } from "./agentBridge";

beforeEach(resetAgentBridgeForTests);

describe("agentBridge", () => {
  it("sends straight through a registered sender", () => {
    const got: string[] = [];
    registerAgentSender("a", (t) => (got.push(t), true));
    expect(sendToAgent("a", "hi")).toBe("sent");
    expect(got).toEqual(["hi"]);
  });
  it("queues for an agent that is not shown and flushes on register", () => {
    expect(sendToAgent("b", "one")).toBe("queued");
    expect(sendToAgent("b", "two")).toBe("queued");
    const got: string[] = [];
    registerAgentSender("b", (t) => (got.push(t), true));
    expect(got).toEqual(["one", "two"]);
  });
  it("keeps a message the socket refused", () => {
    const off = registerAgentSender("c", () => false);
    expect(sendToAgent("c", "x")).toBe("queued");
    off();
    const got: string[] = [];
    registerAgentSender("c", (t) => (got.push(t), true));
    expect(got).toEqual(["x"]);
  });
});
