// Opens the Claude Code session picker from anywhere (palette, Home, New session). The picker's
// code loads the first time it is asked for, so the main bundle does not carry it.
import React, { Suspense, useEffect, useState } from "react";

import { onClaudePickerOpen } from "../../lib/claudeHandoff";

const ClaudeSessionPicker = React.lazy(() => import("./ClaudeSessionPicker"));

export function ClaudePickerHost() {
  const [open, setOpen] = useState(false);
  const [opens, setOpens] = useState(0);
  useEffect(
    () =>
      onClaudePickerOpen(() => {
        setOpens((n) => n + 1);
        setOpen(true);
      }),
    [],
  );
  if (opens === 0) return null;
  return (
    <Suspense fallback={null}>
      <ClaudeSessionPicker key={opens} visible={open} onClose={() => setOpen(false)} />
    </Suspense>
  );
}
