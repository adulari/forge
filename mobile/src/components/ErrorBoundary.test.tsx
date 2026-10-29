import { beforeEach, describe, expect, it, vi } from "vitest";

import { markStartupComplete, resetStartupForTest } from "../lib/startupState";
import { ErrorBoundary } from "./ErrorBoundary";

const markAppError = vi.hoisted(() => vi.fn());

vi.mock("../lib/anonymousTelemetry", () => ({
  markAnonymousTelemetryAppError: markAppError,
}));

vi.mock("../lib/reporting", () => ({ reportBoundaryError: vi.fn() }));

vi.mock("expo-splash-screen", () => ({
  hideAsync: vi.fn(async () => undefined),
}));

vi.mock("./CrashScreen", () => ({ CrashScreen: "CrashScreen" }));

vi.mock("react-native", () => ({
  Platform: { OS: "ios", select: (options: Record<string, unknown>) => options.ios },
  Pressable: "Pressable",
  StyleSheet: { create: (styles: unknown) => styles },
  Text: "Text",
  View: "View",
}));

describe("ErrorBoundary", () => {
  it("records a bounded render error code without forwarding the error", () => {
    const error = new Error("private render details");
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const boundary = new ErrorBoundary({ children: null });

    boundary.componentDidCatch(error);

    expect(markAppError).toHaveBeenCalledWith("react_render");
    expect(markAppError).not.toHaveBeenCalledWith(error);
    consoleError.mockRestore();
  });

  describe("crash classification", () => {
    beforeEach(() => resetStartupForTest());

    it("labels a render error thrown before the first interactive frame as a startup failure", () => {
      const state = ErrorBoundary.getDerivedStateFromError(new Error("provider exploded"));
      expect(state.crash).toMatchObject({ kind: "startup", message: "provider exploded" });
    });

    it("labels it a render error once the app has been interactive", () => {
      markStartupComplete();
      const state = ErrorBoundary.getDerivedStateFromError(new Error("screen exploded"));
      expect(state.crash).toMatchObject({ kind: "render", message: "screen exploded" });
    });

    it("keeps the stack for the diagnostics summary and tolerates non-Error throws", () => {
      expect(ErrorBoundary.getDerivedStateFromError(new Error("x")).crash?.stack).toContain("Error: x");
      expect(ErrorBoundary.getDerivedStateFromError("just a string").crash).toMatchObject({
        message: "just a string",
        stack: null,
      });
    });

    it("renders children until something crashes", () => {
      const boundary = new ErrorBoundary({ children: "child" });
      expect(boundary.render()).toBe("child");
    });
  });
});
