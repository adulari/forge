// tsc-only resolution shim. This project's tsconfig (T0.1-owned, out of this
// task's file scope) doesn't set `moduleSuffixes`, so plain `tsc --noEmit`
// can't resolve an extensionless `./QRScan` import against `QRScan.native.tsx`
// / `QRScan.web.tsx` alone the way Metro does at bundle time. Metro's platform
// resolution always prefers the suffixed file over this bare one on every
// target (native: `.native.tsx`, web: `.web.tsx`) — this file is never
// bundled, it exists solely so the type-checker has something to resolve.
export * from "./QRScan.native";
