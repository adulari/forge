import { useQuery } from "@tanstack/react-query";

import { useAuth } from "./auth";
import { request } from "./api";
import { evidenceListPath, type EvidenceResponse } from "./mediaApi";

const POLL_MS = 5_000;

/** The screenshots a session has captured. Polled while a gallery is on screen, because a browser
 * tool run adds them mid-turn and nothing else announces it. */
export function useEvidenceImages(sessionId: string | null, poll: boolean) {
  const { baseUrl } = useAuth();
  return useQuery<EvidenceResponse>({
    queryKey: ["evidence", baseUrl, sessionId],
    queryFn: () => request<EvidenceResponse>(baseUrl as string, evidenceListPath(sessionId as string)),
    enabled: baseUrl != null && sessionId != null,
    refetchInterval: poll ? POLL_MS : false,
    refetchOnWindowFocus: false,
  });
}
