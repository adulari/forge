import { describe, expect, it, vi } from "vitest";

import { ErrorBoundary } from "./ErrorBoundary";

const markAppError = vi.hoisted(() => vi.fn());

vi.mock("../lib/anonymousTelemetry", () => ({
  markAnonymousTelemetryAppError: markAppError,
}));

vi.mock("expo-splash-screen", () => ({
  hideAsync: vi.fn(async () => undefined),
}));

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
});
