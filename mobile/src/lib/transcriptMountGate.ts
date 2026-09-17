/**
 * Whether the chat screen's real transcript content (session/[id]/index.tsx's `BoundedList`
 * `data`) is safe to mount yet.
 *
 * Android only. `new-session.tsx` reaches this screen via `router.dismissTo(...)` off a
 * `presentation: "modal"` route — dismissing the modal and pushing this screen in one native
 * stack transition. The daemon now runs a freshly created session's first turn itself, so real
 * history rows can land and want to mount into this screen's FlatList WHILE that transition is
 * still in flight — Fabric throws `addViewAt: ... The specified child already has a parent` when
 * the list's cells are created while a view higher up the tree is still being reparented by the
 * dismissing modal, which tears down the whole React instance (blank screen).
 *
 * Withholding the FlatList's real `data` (rendering the same lightweight "connecting…" /
 * "loading…" placeholder it already shows before history has settled) until the screen is both
 * focused and its own transition has finished keeps the heavy transcript from ever mounting
 * mid-reparent, without touching `dismissTo` itself (which fixed an earlier blank-screen bug —
 * see the comment at new-session.tsx around the `handleSubmit` navigation).
 */
export function shouldMountTranscript(input: {
  isAndroid: boolean;
  isFocused: boolean;
  transitionSettled: boolean;
}): boolean {
  if (!input.isAndroid) return true;
  return input.isFocused && input.transitionSettled;
}
