// Activation thresholds for the tab pager's drag.
//
// Plain numbers in their own module so the relationships between them can be asserted — they are the
// difference between a swipe that pages and a swipe the list underneath steals. TabPager closes over
// them inside worklets, which is safe: constants are captured by value.

/** Horizontal travel before the pager claims the drag. */
export const ACTIVATE_X = 20;

/** Vertical travel that cancels the pager and gives the drag back to a scroll view. */
export const FAIL_Y = 24;

/**
 * SessionCard's swipe-to-archive pan, for reference only — it is a DESCENDANT of the pager, and
 * gesture-handler cancels an ancestor once a descendant activates, so a lower number here means a
 * card keeps its own drag. Recorded so the relationship is checked rather than remembered.
 */
export const SESSION_CARD_ACTIVATE_X = 10;

/** Fraction of the screen a drag must cross to complete on distance alone. */
export const COMMIT_FRACTION = 0.32;

/** Or this much horizontal speed, so a short flick still completes. */
export const COMMIT_VELOCITY = 520;

/** How much of a drag past the first/last tab is shown, as resistance rather than a dead stop. */
export const OVERSCROLL = 0.2;
