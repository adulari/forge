// Mounted INSIDE AppLock: a notice's text is exactly what the generic push hides, so it must not
// draw over the biometric lock screen.
import React, { useEffect, useRef } from "react";

import { markNoticesSeen, showBanner, useNoticesStore } from "../../lib/notices/store";
import { NoticeBanner } from "./NoticeBanner";
import { useOpenNotice } from "./useOpenNotice";

const AUTO_DISMISS_MS = 8000;

export function NoticeBannerHost() {
  const { banner } = useNoticesStore();
  const open = useOpenNotice();
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (timer.current) clearTimeout(timer.current);
    if (!banner) return;
    timer.current = setTimeout(() => showBanner(null), AUTO_DISMISS_MS);
    return () => {
      if (timer.current) clearTimeout(timer.current);
    };
  }, [banner]);

  if (!banner) return null;
  const newest = banner.notices[0];
  return (
    <NoticeBanner
      notices={banner.notices}
      onOpen={() => {
        showBanner(null);
        // One notice opens its own session; a burst opens the list so none is skipped.
        if (banner.notices.length === 1) open(banner.serverId, newest);
        else {
          markNoticesSeen(banner.serverId, newest);
          open(banner.serverId);
        }
      }}
      onDismiss={() => {
        markNoticesSeen(banner.serverId, newest);
        showBanner(null);
      }}
    />
  );
}
