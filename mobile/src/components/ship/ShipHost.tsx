// Opens the Ship sheet from anywhere (session menu, Review dock, command palette).
import React, { useEffect, useState } from "react";

import { onShipOpen } from "../../lib/shipRequest";
import { ShipSheet } from "./ShipSheet";

export function ShipHost() {
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [visible, setVisible] = useState(false);
  useEffect(
    () =>
      onShipOpen((id) => {
        setSessionId(id);
        setVisible(true);
      }),
    [],
  );
  return <ShipSheet visible={visible} sessionId={sessionId} onClose={() => setVisible(false)} />;
}
