// Per-server "last time the app was in front of you", the only state the away card keeps. It is
// stored under the token-free server key id (a baseUrl carries the pairing token) and written on
// return, on leaving, and on a slow heartbeat while the app is visible so a killed app still
// leaves a recent stamp.
import AsyncStorage from "@react-native-async-storage/async-storage";

import { serverKeyId } from "../serverKeyId";

const PREFIX = "forge.brief.lastVisit.v1:";

function key(baseUrl: string): string {
  return `${PREFIX}${serverKeyId(baseUrl)}`;
}

export async function readLastVisit(baseUrl: string): Promise<number | null> {
  try {
    const raw = await AsyncStorage.getItem(key(baseUrl));
    const value = raw == null ? NaN : Number(raw);
    return Number.isFinite(value) && value > 0 ? value : null;
  } catch {
    return null;
  }
}

export async function stampVisit(baseUrl: string, nowSecs = Math.floor(Date.now() / 1000)): Promise<void> {
  try {
    await AsyncStorage.setItem(key(baseUrl), String(nowSecs));
  } catch {
    // A missing stamp only means no card next time.
  }
}
