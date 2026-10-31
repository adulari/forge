import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { request } from "./api";
import { useAuth } from "./auth";
import type { ImportAgent, ImportReply, ImportScan } from "./importSessions";

function getImportScan(baseUrl: string): Promise<ImportScan> {
  return request(baseUrl, "/api/import/scan");
}

function postImport(
  baseUrl: string,
  items: { agent: ImportAgent; id: string }[],
  fallbackCwd?: string,
): Promise<ImportReply> {
  return request(
    baseUrl,
    "/api/import",
    { method: "POST", body: JSON.stringify({ items, fallback_cwd: fallbackCwd || undefined }) },
    120_000,
  );
}



const scanKey = (baseUrl: string | null) => ["import-scan", baseUrl] as const;

/** What can be imported from the host the app is connected to. Always read fresh. */
export function useImportScan() {
  const { baseUrl } = useAuth();
  return useQuery<ImportScan>({
    queryKey: scanKey(baseUrl),
    queryFn: () => getImportScan(baseUrl as string),
    enabled: baseUrl != null,
    staleTime: 0,
    gcTime: 0,
    retry: false,
  });
}

export function useImportSessions() {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation<ImportReply, Error, { items: { agent: ImportAgent; id: string }[]; fallbackCwd?: string }>({
    mutationFn: ({ items, fallbackCwd }) => postImport(baseUrl as string, items, fallbackCwd),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["sessions"] });
      void queryClient.invalidateQueries({ queryKey: scanKey(baseUrl) });
    },
  });
}
