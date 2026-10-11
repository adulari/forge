import { router } from "expo-router";
import { useCallback } from "react";

import { useAuth } from "../../lib/auth";
import { markNoticesSeen } from "../../lib/notices/store";
import { sessionIdOf, type Notice } from "../../lib/notices/core";

/** Switch to the notice's host if needed, then open its session (or the list when it has none). */
export function useOpenNotice() {
  const { activeServerId, setActive } = useAuth();
  return useCallback(
    (serverId: string, notice?: Notice) => {
      if (notice) markNoticesSeen(serverId, notice);
      if (serverId !== activeServerId) setActive(serverId);
      const sessionId = notice ? sessionIdOf(notice) : null;
      router.push(sessionId ? `/session/${sessionId}` : "/notices");
    },
    [activeServerId, setActive],
  );
}
