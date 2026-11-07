import React from "react";
// @ts-expect-error -- react-dom ships no type declarations in this app (react-native-web pulls it in)
import * as server from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { SHIP_OFF_MESSAGE, UPDATE_HOST_MESSAGE } from "../../lib/prAccessMessages";
import { ShipSheet } from "./ShipSheet";

const renderToStaticMarkup = (server as { renderToStaticMarkup: (el: React.ReactElement) => string }).renderToStaticMarkup;

type Props = Record<string, unknown> & { children?: React.ReactNode };
const gate = vi.hoisted(() => ({ value: { allowed: false, message: null as string | null } }));
const mutate = vi.hoisted(() => vi.fn());

vi.mock("react-native", () => {
  const box = (tag: string) => {
    const Box = (props: Props) => React.createElement(tag, null, props.children);
    Box.displayName = tag;
    return Box;
  };
  return {
    Platform: { OS: "web", select: (o: Record<string, unknown>) => o.web ?? o.default },
    StyleSheet: { create: (s: unknown) => s, hairlineWidth: 1 },
    Linking: { openURL: vi.fn() },
    Text: box("span"),
    View: box("div"),
  };
});
vi.mock("../../lib/prAccess", () => ({ useShipGate: () => gate.value }));
vi.mock("../../lib/queriesPr", () => ({
  useShipSession: () => ({ mutate, isPending: false }),
}));
vi.mock("../../theme/ThemeProvider", () => ({
  useTokens: () => ({ ink: "ink", ink2: "ink2", ink3: "ink3", accent: "accent", danger: "danger", border: "border" }),
}));
vi.mock("../ds/Badge", () => ({ Badge: () => null }));
vi.mock("../ds/Button", () => ({ Button: (props: { label: string }) => React.createElement("button", null, props.label) }));
vi.mock("../ds/Input", () => ({ Input: () => null }));
vi.mock("../ds/Sheet", () => ({ Sheet: (props: Props) => React.createElement("section", null, props.children) }));

const render = () =>
  renderToStaticMarkup(React.createElement(ShipSheet, { visible: true, onClose: () => {}, sessionId: "s1" }));

beforeEach(() => mutate.mockReset());

describe("ShipSheet over Anywhere", () => {
  it("names the host setting instead of asking for a direct connection when ship is off", () => {
    gate.value = { allowed: false, message: SHIP_OFF_MESSAGE };
    const html = render();
    expect(html).toContain("Ship over Anywhere is off on this host");
    expect(html).toContain("allow_ship_over_relay");
    expect(html).not.toContain("direct connection");
    expect(mutate).not.toHaveBeenCalled();
  });

  it("tells an older host to be updated", () => {
    gate.value = { allowed: false, message: UPDATE_HOST_MESSAGE };
    expect(render()).toContain("update Forge on the host");
  });

  it("shows the ship flow, not a refusal, when the host allows ship over the relay", () => {
    gate.value = { allowed: true, message: null };
    const html = render();
    expect(html).not.toContain("Ship over Anywhere is off");
    expect(html).not.toContain("too old");
    expect(html).not.toContain("direct connection");
  });
});
