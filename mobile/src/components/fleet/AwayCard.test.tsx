import React from "react";
// @ts-expect-error -- react-dom ships no type declarations in this app (react-native-web pulls it in)
import * as server from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { Brief } from "../../lib/brief/core";
import { AwayCardView } from "./AwayCard";

type Props = Record<string, unknown> & { children?: React.ReactNode };

vi.mock("react-native", () => {
  const box = (tag: string) => {
    const Box = (props: Props) => React.createElement(tag, { "aria-label": props.accessibilityLabel as string | undefined }, props.children);
    Box.displayName = tag;
    return Box;
  };
  return {
    Platform: { OS: "web", select: (o: Record<string, unknown>) => o.web ?? o.default },
    StyleSheet: { create: (s: unknown) => s, hairlineWidth: 1 },
    AppState: { addEventListener: () => ({ remove: () => undefined }), currentState: "active" },
    Text: box("span"),
    View: box("div"),
    Pressable: box("button"),
  };
});
vi.mock("lucide-react-native", () => ({ X: () => null }));
vi.mock("@tanstack/react-query", () => ({ useQuery: () => ({ data: undefined }) }));
vi.mock("../ds/Card", () => ({ Card: (p: Props) => React.createElement("section", null, p.children) }));
vi.mock("../ds/IconButton", () => ({ IconButton: (p: Props) => React.createElement("button", { "aria-label": p.accessibilityLabel as string }) }));
vi.mock("../ds/StatusDot", () => ({ StatusDot: () => null }));
vi.mock("../../theme/ThemeProvider", () => ({ useTokens: () => new Proxy({}, { get: () => "#000" }) }));
vi.mock("../../lib/auth", () => ({ useAuth: () => ({ baseUrl: "https://host/tok" }) }));
vi.mock("../../lib/useScopedFleet", () => ({ useOpenSession: () => () => undefined }));
vi.mock("../../lib/brief/store", () => ({ readLastVisit: async () => null, stampVisit: async () => undefined }));
vi.mock("../../lib/brief/api", () => ({ getBrief: async () => undefined }));
vi.mock("../../lib/platform", () => ({ isWeb: false }));

const renderToStaticMarkup = (server as { renderToStaticMarkup: (el: React.ReactElement) => string }).renderToStaticMarkup;

const brief: Brief = {
  since: 0,
  now: 6 * 3600,
  totals: { sessions: 3, finished: 1, failed: 1, waiting: 1, interrupted: 0, working: 0, changesReady: 1, spendUsd: 3.1 },
  sessions: [
    { id: "w", title: "Migrate schema", project: "forge", outcome: "waiting", reason: null, waitingSince: 6 * 3600 - 2400, lastActivity: 6 * 3600 - 2400, summary: "", spendUsd: 0, unlanded: null },
    { id: "f", title: "Fix flaky test", project: "forge", outcome: "failed", reason: "provider 503", waitingSince: null, lastActivity: 100, summary: "", spendUsd: 0.4, unlanded: null },
    { id: "d", title: "Add flag", project: "forge", outcome: "finished", reason: null, waitingSince: null, lastActivity: 90, summary: "Added it", spendUsd: 2.7, unlanded: { added: 12, removed: 3 } },
  ],
};

describe("AwayCardView", () => {
  it("renders the window, the headline and one line per session with its state", () => {
    const html = renderToStaticMarkup(<AwayCardView brief={brief} onOpen={() => undefined} onDismiss={() => undefined} />);
    expect(html).toContain("While you were away (6h)");
    expect(html).toContain("1 finished, 1 failed, 1 waiting on you, 1 with changes ready to land, $3.10 spent");
    expect(html).toContain("was waiting on you for 40 min");
    expect(html).toContain("failed: provider 503");
    expect(html).toContain("+12/-3 to land · $2.70");
    expect(html).toContain('aria-label="Dismiss summary"');
  });

  it("is a snapshot of the return: later wall-clock time does not grow the wait or revive 'still working'", () => {
    const working = { ...brief, sessions: [...brief.sessions, { ...brief.sessions[2], id: "k", title: "Refactor", outcome: "working" as const }] };
    const render = () => renderToStaticMarkup(<AwayCardView brief={working} onOpen={() => undefined} onDismiss={() => undefined} />);
    vi.useFakeTimers();
    try {
      vi.setSystemTime(new Date((6 * 3600) * 1000));
      const atReturn = render();
      vi.setSystemTime(new Date((9 * 3600) * 1000));
      const later = render();
      expect(later).toBe(atReturn);
      expect(later).toContain("was waiting on you for 40 min");
      expect(later).toContain("was still working");
      expect(later).not.toContain(" still working</span>");
    } finally {
      vi.useRealTimers();
    }
  });
});
