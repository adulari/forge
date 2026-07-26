// Activation thresholds for the tab-switch swipe.
//
// Plain numbers in their own module so the relationships between them can be asserted — they are the
// difference between a swipe that changes tab and a swipe the list underneath steals.

/** Horizontal travel before the swipe claims the drag. */
export const ACTIVATE_X = 20;

/** Vertical travel that cancels the swipe and gives the drag back to a scroll view. */
export const FAIL_Y = 24;

/**
 * SessionCard's swipe-to-archive pan, for reference only — it is a DESCENDANT of this gesture, and
 * gesture-handler cancels an ancestor once a descendant activates, so a lower number here means a
 * card keeps its own drag. Recorded so the relationship is checked rather than remembered.
 */
export const SESSION_CARD_ACTIVATE_X = 10;

/** Horizontal distance that completes a tab change on distance alone. */
export const COMMIT_DISTANCE = 64;

/** Or this much horizontal speed, so a short flick still completes. */
export const COMMIT_VELOCITY = 520;
