// Pure half of `desktopNotify.ts` — the privacy gate that decides what an OS-level surface
// (native notification, menu-bar extra) is allowed to say. Split out for the same reason
// `appVersionCore.ts` is: `desktopNotify.ts` reaches `notify()` → `platform.ts` → `react-native`,
// which the test runner cannot parse, and this logic is exactly the part that needs asserting.
//
// The rule (docs/design/machined Desktop.dc.html L976: "generic when locked — Anywhere pushes
// reveal nothing"): a notification is drawn by the OS, outside the app's lock, and for an
// Anywhere-routed session it describes work that only exists on someone else's machine behind
// end-to-end encryption. So when the content is locked, nothing about it leaves the app.

export interface DecisionNotice {
  /** Real session title. Never reaches the OS when `locked`. */
  sessionTitle: string;
  /** What is being asked, e.g. `Permission: overwrite .env.staging`. Never reaches the OS when
   * `locked`. */
  detail?: string;
  /** True when the content must not leave the app: the session is routed over Forge Anywhere
   * (its contents are end-to-end encrypted and the relay never sees them), or the app is
   * locked behind the biometric gate. */
  locked: boolean;
}

export interface NotificationCopy {
  title: string;
  body: string;
}

/** Whether a server's transport means its session content is off-limits to OS surfaces. */
export function isContentLocked(transport?: "direct" | "anywhere"): boolean {
  return transport === "anywhere";
}

export function decisionCopy(notice: DecisionNotice): NotificationCopy {
  if (notice.locked) {
    return {
      title: "Forge needs you",
      // Deliberately says nothing about which session, which host, or what is being asked.
      body: "A session is waiting for a decision. Open Forge to review.",
    };
  }
  return {
    title: `${notice.sessionTitle} needs you`,
    body: notice.detail ?? "A decision is waiting.",
  };
}

/** Tray rows live in the same public surface as notifications, so they get the same gate. */
export function redactTrayTitle(title: string, locked: boolean): string {
  return locked ? "Session" : title;
}
