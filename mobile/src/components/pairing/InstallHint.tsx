// tsc-only resolution shim; Metro always prefers InstallHint.web.tsx on web, and the PWA install
// prompt means nothing on native, where this renders nothing.
export function InstallHint(): null {
  return null;
}
