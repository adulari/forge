import React, { useState } from "react";

import type { AccountSyncCategory } from "../../lib/api";
import { KEYS_CONFIRM_MESSAGE, KEYS_CONFIRM_TITLE, needsKeysConfirmation } from "../../lib/accountSync";
import { ConfirmDialog } from "../ds/ConfirmDialog";

/** Wraps a toggle so turning key sync on goes through a confirmation that opens on Cancel,
 * matching the terminal menu and the CLI. Render `dialog` next to the switches. */
export function useConfirmedSyncToggle(apply: (category: AccountSyncCategory, enabled: boolean) => void) {
  const [pending, setPending] = useState<AccountSyncCategory | null>(null);
  const request = (category: AccountSyncCategory, enabled: boolean) => {
    if (needsKeysConfirmation(category, enabled)) setPending(category);
    else apply(category, enabled);
  };
  const dialog = (
    <ConfirmDialog
      visible={pending != null}
      title={KEYS_CONFIRM_TITLE}
      message={KEYS_CONFIRM_MESSAGE}
      confirmLabel="Turn on key sync"
      destructive
      onConfirm={() => {
        const category = pending;
        setPending(null);
        if (category) apply(category, true);
      }}
      onCancel={() => setPending(null)}
    />
  );
  return { request, dialog };
}
