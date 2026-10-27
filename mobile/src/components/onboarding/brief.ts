// The desktop app's cheap, always-on question to its daemon: should setup open, and does Claude
// Code need reconnecting? Only asked on the desktop shell, and only when the daemon is on this
// machine (it answers 403 anywhere else, which reads as "nothing to say").
import { useQuery } from "@tanstack/react-query";

import { useAuth } from "../../lib/auth";
import { getOnboardingBrief } from "../../lib/onboarding/api";
import type { Brief } from "../../lib/onboarding/types";
import { isTauri } from "../../lib/platform";

const RECHECK_MS = 5 * 60 * 1000;

export function useOnboardingBrief(): Brief | null {
  const { baseUrl } = useAuth();
  const query = useQuery({
    queryKey: ["onboarding-brief", baseUrl],
    queryFn: () => getOnboardingBrief(baseUrl as string),
    enabled: isTauri && baseUrl != null,
    retry: false,
    staleTime: RECHECK_MS,
    refetchInterval: RECHECK_MS,
    refetchOnWindowFocus: false,
  });
  return query.data ?? null;
}
