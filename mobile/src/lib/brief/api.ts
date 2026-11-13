import { request } from "../api";
import { parseBrief, type Brief } from "./core";

/** `GET /api/brief`; over Anywhere the same path is the bridge route `brief`. */
export async function getBrief(baseUrl: string, since: number): Promise<Brief> {
  return parseBrief(await request<unknown>(baseUrl, `/api/brief?since=${Math.max(0, Math.floor(since))}`));
}
