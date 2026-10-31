// Routes first-run setup sends people to and that must stay reachable while setup is open, or
// the layout's "bounce to /onboarding" redirect undoes the navigation on the same render.
const EXEMPT = /^\/(onboarding|import)(\/|$)/;

export function isOnboardingExempt(pathname: string): boolean {
  return EXEMPT.test(pathname);
}
