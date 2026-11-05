// Commands that arrive from outside the shell (the Mod+J terminal chord, the View menu) while the
// IDE is on screen. The shell registers one listener for as long as it is mounted and visible.
type Listener = () => void;

let terminalListener: Listener | null = null;

export function onIdeTerminalToggle(listener: Listener): () => void {
  terminalListener = listener;
  return () => {
    if (terminalListener === listener) terminalListener = null;
  };
}

/** True when the IDE took the request. */
export function requestIdeTerminalToggle(): boolean {
  if (!terminalListener) return false;
  terminalListener();
  return true;
}

let sendReviewListener: Listener | null = null;

/** The review draft bar registers while it has comments to send. */
export function onIdeSendReview(listener: Listener): () => void {
  sendReviewListener = listener;
  return () => {
    if (sendReviewListener === listener) sendReviewListener = null;
  };
}

/** True when a review was waiting and went out. */
export function requestIdeSendReview(): boolean {
  if (!sendReviewListener) return false;
  sendReviewListener();
  return true;
}
