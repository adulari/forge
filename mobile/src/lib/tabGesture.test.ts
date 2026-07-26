import { describe, expect, it } from "vitest";

import {
  ACTIVATE_X,
  COMMIT_DISTANCE,
  COMMIT_VELOCITY,
  FAIL_Y,
  SESSION_CARD_ACTIVATE_X,
} from "./tabGesture";

describe("tab swipe activation thresholds", () => {
  // The bug this pins: the swipe first shipped needing |dx| > 28 before |dy| > 15, a ratio of nearly
  // 2:1 horizontal-to-vertical just to begin. A thumb swipe arcs, so an ordinary curved swipe crossed
  // the vertical limit first, the gesture failed, and the list underneath scrolled instead.
  it("cancels later vertically than it activates horizontally", () => {
    expect(FAIL_Y).toBeGreaterThan(ACTIVATE_X);
  });

  // Anything under roughly 45 degrees should change tab rather than scroll. Stated as the ratio
  // because that, not either number alone, is what the thumb experiences.
  it("switches for intent that is only roughly sideways", () => {
    expect(ACTIVATE_X / FAIL_Y).toBeLessThan(1);
  });

  // SessionCard's archive pan is a descendant, so it activates first and gesture-handler cancels the
  // ancestor. If this gesture ever activated at or below the card's threshold, a card drag on Fleet
  // would race the tab change instead of winning it.
  it("stays clear of the session card's own swipe", () => {
    expect(ACTIVATE_X).toBeGreaterThan(SESSION_CARD_ACTIVATE_X);
  });

  it("commits well past the point the gesture activates", () => {
    // Otherwise activating would be indistinguishable from committing, and every claimed drag would
    // change tab — including one the user began and thought better of.
    expect(COMMIT_DISTANCE).toBeGreaterThan(ACTIVATE_X * 2);
    expect(COMMIT_VELOCITY).toBeGreaterThan(300);
  });
});
