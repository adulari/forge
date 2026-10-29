import { Linking } from "react-native";
import { describe, expect, it, vi } from "vitest";

import { canOpenMicSettings, isMicDenied, MIC_DENIED_MESSAGE, MicDeniedError, openMicSettings } from "./micAccess";

vi.mock("react-native", () => ({
  Platform: { OS: "ios" },
  Linking: { openSettings: vi.fn(async () => undefined) },
}));

describe("microphone access", () => {
  it("words a denial as a state the user can fix, not a failure", () => {
    expect(new MicDeniedError().message).toBe("Microphone access is off");
    expect(MIC_DENIED_MESSAGE).toBe("Microphone access is off");
  });

  it("recognises a native denial and the browser's permission errors", () => {
    expect(isMicDenied(new MicDeniedError())).toBe(true);
    expect(isMicDenied(Object.assign(new Error("Permission denied"), { name: "NotAllowedError" }))).toBe(true);
    expect(isMicDenied(Object.assign(new Error("x"), { name: "SecurityError" }))).toBe(true);
  });

  it("does not mistake other failures for a denial", () => {
    expect(isMicDenied(Object.assign(new Error("no mic"), { name: "NotFoundError" }))).toBe(false);
    expect(isMicDenied(new Error("recording produced no audio"))).toBe(false);
    expect(isMicDenied(null)).toBe(false);
    expect(isMicDenied("NotAllowedError")).toBe(false);
  });

  it("opens the system settings on native", async () => {
    expect(canOpenMicSettings()).toBe(true);
    await openMicSettings();
    expect(Linking.openSettings).toHaveBeenCalledTimes(1);
  });
});
