// Barrel for the push seam (ARCHITECTURE.md §2 platform escape hatches):
// Metro resolves this specifier to `push.web.ts` on web and `push.ts`
// (native no-op) elsewhere. Callers only ever import from "../../lib/push".
export * from "./push";
