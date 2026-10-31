// Images the daemon serves for a session (crates/forge-cli/src/serve_media.rs): a project image
// by path, and the screenshots the session captured as evidence. The URLs go straight into an
// <Image>: the daemon token rides in the base URL, so no header is needed.
export interface EvidenceImage {
  file: string;
  ts: string;
  source: string;
  caption: string | null;
  url: string | null;
  size: number;
}

export interface EvidenceResponse {
  items: EvidenceImage[];
}

function query(params: Record<string, string | number | undefined>): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined) search.set(key, String(value));
  }
  return `?${search.toString()}`;
}

/** A project image, or one in the session's evidence folder, by the path an agent printed.
 * `width` asks for a downsampled copy (thumbnails). */
export function workspaceImageUrl(baseUrl: string, session: string, path: string, width?: number): string {
  return `${baseUrl}/api/workspace/image${query({ session, path, w: width })}`;
}

/** A `.pdf` in the session's workspace, for a frame (crates/forge-cli/src/serve_workspace_pdf.rs). */
export function workspacePdfUrl(baseUrl: string, session: string, path: string): string {
  return `${baseUrl}/api/workspace/pdf${query({ session, path })}`;
}

export function evidenceImageUrl(baseUrl: string, session: string, file: string, width?: number): string {
  return `${baseUrl}/api/evidence/image${query({ session, file, w: width })}`;
}

export function evidenceListPath(session: string): string {
  return `/api/evidence${query({ session })}`;
}

/** The path a browser or device screenshot result names: "screenshot written to <path> (…)". */
export function screenshotPathOf(result: string | null | undefined): string | null {
  const match = /screenshot written to (.+?) \(\d+ bytes/.exec(result ?? "");
  return match ? match[1] : null;
}
