import { useMemo } from "react";

import { type FollowUpSetting, followUpFromConfig } from "./followUp";
import { useConfig } from "./queries";

export function useFollowUpSetting(): FollowUpSetting {
  const { data } = useConfig();
  return useMemo(() => followUpFromConfig(data?.fields), [data?.fields]);
}
