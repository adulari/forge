// Guards the routing invariant documented at the top of src/app/(tabs)/_layout.tsx: every file
// in the (tabs) group must be a real tab in BOTH navigators. A (tabs) route with no
// <NativeTabs.Trigger> is not hidden, it is deleted — expo-router files untriggered children
// into `protectedScreens` and `useSortedScreens` drops them from the navigator — so on iOS
// (the only platform taking the NativeTabs branch) navigating to it does nothing at all while
// the same tap works on Android, web and desktop. That is exactly how /floor and /plans became
// dead buttons on iPhone; they now live in the root stack and must stay there.
//
// This is a source-text assertion rather than a render test because the failure is in route
// REGISTRATION, which only happens inside a real expo-router tree. It lives in lib/ rather than
// beside its subject because every file under src/app is itself a route — a test file there
// would ship as one.
//
// The reference directive is file-local on purpose: the app project has no ambient Node types
// (nothing else in src/ may touch the filesystem), and putting "node" in tsconfig's `types`
// would leak Node globals — a `setTimeout` returning NodeJS.Timeout instead of a number — into
// every React Native module. @types/node itself comes in with vitest.
/// <reference types="node" />
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

const appDir = join(dirname(fileURLToPath(import.meta.url)), "..", "app");
const tabsDir = join(appDir, "(tabs)");
const layoutSource = readFileSync(join(tabsDir, "_layout.tsx"), "utf8");

function routeFiles(): string[] {
  return readdirSync(tabsDir, { withFileTypes: true })
    .filter((entry) => entry.isFile() && /\.tsx$/.test(entry.name) && entry.name !== "_layout.tsx")
    .map((entry) => entry.name.replace(/\.tsx$/, ""))
    .sort();
}

function declaredNames(component: string): string[] {
  const matches = layoutSource.matchAll(new RegExp(`<${component}\\s+name="([^"]+)"`, "g"));
  return [...matches].map((match) => match[1]).sort();
}

describe("(tabs) route registration", () => {
  it("declares a NativeTabs trigger for every route in the group", () => {
    expect(declaredNames("NativeTabs\\.Trigger")).toEqual(routeFiles());
  });

  it("declares a Tabs.Screen for every route in the group", () => {
    expect(declaredNames("Tabs\\.Screen")).toEqual(routeFiles());
  });

  // `hidden` reads like Tabs.Screen's `href: null` and is not: navigating to a hidden trigger
  // throws in dev and silently lands on tab 0 in release (NativeBottomTabsNavigator.js's
  // `visibleFocusedTabIndex < 0` branch). Only Trigger itself is checked — Badge's own `hidden`
  // prop is the ordinary way to drop the Inbox dot.
  it("marks no trigger hidden", () => {
    expect(layoutSource).not.toMatch(/<NativeTabs\.Trigger\s[^>]*\bhidden\b/);
  });

  it("keeps the push-only screens in the root stack so /floor and /plans stay reachable", () => {
    const rootRoutes = readdirSync(appDir, { withFileTypes: true })
      .filter((entry) => entry.isFile())
      .map((entry) => entry.name);
    expect(rootRoutes).toContain("floor.tsx");
    expect(rootRoutes).toContain("plans.tsx");
  });
});
