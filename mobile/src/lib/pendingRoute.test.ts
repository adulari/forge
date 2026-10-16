import { expect, it } from "vitest";

import { rememberPendingRoute, takePendingRoute } from "./pendingRoute";

it("returns the deep-linked route once, ignoring the root and connect", () => {
  rememberPendingRoute("/");
  expect(takePendingRoute()).toBeNull();
  rememberPendingRoute("/session/abc");
  rememberPendingRoute("/connect");
  expect(takePendingRoute()).toBe("/session/abc");
  expect(takePendingRoute()).toBeNull();
});
