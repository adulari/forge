// Reviewed marks, persisted per viewer. A mark is `session+path → content hash`; changeQueueModel
// only honours it while the file still has that hash, so an agent editing the file again clears it
// without any bookkeeping here. Pruning on top keeps the blob from growing.
import AsyncStorage from "@react-native-async-storage/async-storage";
import { useSyncExternalStore } from "react";

import { markReviewed, pruneReviewed, unmarkReviewed, type ReviewedMarks } from "./changeQueueModel";

const STORAGE_KEY = "forge.ideReviewed.v1";
const MAX_MARKS = 2000;

let marks: ReviewedMarks = {};
let hydrated = false;
let hydration: Promise<void> | null = null;
const listeners = new Set<() => void>();

export function parseReviewed(raw: string | null): ReviewedMarks {
  if (!raw) return {};
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    const out: Record<string, string> = {};
    for (const [key, value] of Object.entries(parsed)) {
      if (typeof value === "string" && key.includes("\0")) out[key] = value;
    }
    return out;
  } catch {
    return {};
  }
}

function set(next: ReviewedMarks): void {
  if (next === marks) return;
  marks = next;
  for (const listener of listeners) listener();
  const entries = Object.entries(next);
  // Newest marks sit last in insertion order; drop the oldest past the cap.
  const capped = entries.length > MAX_MARKS ? Object.fromEntries(entries.slice(entries.length - MAX_MARKS)) : next;
  void AsyncStorage.setItem(STORAGE_KEY, JSON.stringify(capped)).catch(() => undefined);
}

function ensureLoaded(): Promise<void> {
  if (hydrated) return Promise.resolve();
  hydration ??= AsyncStorage.getItem(STORAGE_KEY)
    .then((raw) => {
      hydrated = true;
      // Marks made before hydration finished win over the stored ones.
      marks = { ...parseReviewed(raw), ...marks };
      for (const listener of listeners) listener();
    })
    .catch(() => {
      hydrated = true;
    });
  return hydration;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  void ensureLoaded();
  return () => {
    listeners.delete(listener);
  };
}

const snapshot = () => marks;

export function useReviewedMarks(): ReviewedMarks {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

export function readReviewedMarks(): ReviewedMarks {
  return marks;
}

export function markFileReviewed(sessionId: string, path: string, hash: string): void {
  set(markReviewed(marks, sessionId, path, hash));
}

export function unmarkFileReviewed(sessionId: string, path: string): void {
  set(unmarkReviewed(marks, sessionId, path));
}

export function pruneReviewedMarks(live: ReadonlyMap<string, ReadonlyMap<string, string>>): void {
  set(pruneReviewed(marks, live));
}

export function resetReviewedForTests(): void {
  marks = {};
  hydrated = false;
  hydration = null;
}
