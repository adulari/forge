// Marks a subtree as a PEEK — the copy of a neighbouring tab that TabPager draws beside the
// selected one during a swipe. It is on screen for the length of a drag and then thrown away.
//
// Without this, a throwaway instance behaves like a real screen arriving: it forces the refetch that
// `useSessions` asks for on every mount, so every swipe fired a network round trip whose only
// visible effect was the reload flicker it caused. Anything that should happen when a user ARRIVES
// at a screen, rather than when pixels of it are briefly revealed, belongs behind this flag.
import React from "react";

const PeekContext = React.createContext(false);

export function PeekProvider({ children }: { children: React.ReactNode }) {
  return <PeekContext.Provider value={true}>{children}</PeekContext.Provider>;
}

/** True inside a peeked copy of a tab. False in the tab the user is actually on. */
export function useIsPeeking(): boolean {
  return React.useContext(PeekContext);
}
