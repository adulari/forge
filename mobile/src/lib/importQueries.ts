import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { request } from "./api";
import { useAuth } from "./auth";
import { IMPORT_BATCH, type ImportAgent, type ImportReply, type ImportScan, chunk, mergeReplies } from "./importSessions";

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



async function importInBatches(
  baseUrl: string,
  items: { agent: ImportAgent; id: string }[],
  fallbackCwd?: string,
): Promise<ImportReply> {
  const replies: ImportReply[] = [];
  for (const batch of chunk(items, IMPORT_BATCH)) replies.push(await postImport(baseUrl, batch, fallbackCwd));
  return mergeReplies(replies);
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
    mutationFn: ({ items, fallbackCwd }) => importInBatches(baseUrl as string, items, fallbackCwd),
    // After a timeout the host may still have finished: a rescan shows what really came over,
    // and importing is idempotent per conversation, so trying again is safe.
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["sessions"] });
      void queryClient.invalidateQueries({ queryKey: scanKey(baseUrl) });
    },
  });
}
