import { describe, expect, it } from "vitest";

import {
  ACTIVATE_X,
  COMMIT_FRACTION,
  COMMIT_VELOCITY,
  FAIL_Y,
  OVERSCROLL,
  SESSION_CARD_ACTIVATE_X,
} from "./tabGesture";

describe("tab pager activation thresholds", () => {
  // The bug this pins: the pager first shipped needing |dx| > 28 before |dy| > 15, a ratio of nearly
  // 2:1 horizontal-to-vertical just to begin. A thumb swipe arcs, so an ordinary curved swipe crossed
  // the vertical limit first, the gesture failed, and the list underneath scrolled instead.
  it("cancels later vertically than it activates horizontally", () => {
    expect(FAIL_Y).toBeGreaterThan(ACTIVATE_X);
  });

  // Anything under roughly 45 degrees should page rather than scroll. Stated as the ratio because
  // that, not either number alone, is what the thumb experiences.
  it("pages for intent that is only roughly sideways", () => {
    expect(ACTIVATE_X / FAIL_Y).toBeLessThan(1);
  });

  // SessionCard's archive pan is a descendant, so it activates first and gesture-handler cancels the
  // ancestor. If the pager ever activated at or below the card's threshold, a card drag on Fleet
  // would race the tab change instead of winning it.
  it("stays clear of the session card's own swipe", () => {
    expect(ACTIVATE_X).toBeGreaterThan(SESSION_CARD_ACTIVATE_X);
  });

  it("keeps commit within reach of a normal drag", () => {
    // A third of the screen, not half: a swipe that has clearly gone somewhere should land.
    expect(COMMIT_FRACTION).toBeGreaterThan(0.2);
    expect(COMMIT_FRACTION).toBeLessThan(0.5);
    // Fast enough that a flick is not mistaken for a scroll, slow enough that a deliberate flick
    // qualifies.
    expect(COMMIT_VELOCITY).toBeGreaterThan(300);
  });

  it("resists at the ends without stopping dead", () => {
    expect(OVERSCROLL).toBeGreaterThan(0);
    expect(OVERSCROLL).toBeLessThan(0.5);
  });
});
