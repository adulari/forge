// Opens the Local CI sheet from anywhere (session menu, command palette, the blocked banner).
import React, { useEffect, useState } from "react";

import { onCiOpen } from "../../lib/ciRequest";
import { LocalCiSheet } from "./LocalCiSheet";

export function CiHost() {
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [visible, setVisible] = useState(false);
  useEffect(
    () =>
      onCiOpen((id) => {
        setSessionId(id);
        setVisible(true);
      }),
    [],
  );
  return <LocalCiSheet visible={visible} sessionId={sessionId} onClose={() => setVisible(false)} />;
}
