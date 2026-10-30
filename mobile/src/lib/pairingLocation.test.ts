import { afterEach, describe, expect, it, vi } from "vitest";
import { readPairingFromLocation, stripPairingFromLocation } from "./pairingLocation";

vi.mock("react-native", () => ({ Platform: { OS: "web" } }));

function stubWindow(hash: string, search = "") {
  const replaceState = vi.fn();
  vi.stubGlobal("window", {
    location: { origin: "https://h:7420", hash, search, pathname: "/connect" },
    history: { state: null, replaceState },
  });
  return replaceState;
}

describe("pairing token in the address bar", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("reads a pairing fragment and strips it from history", () => {
    const replaceState = stubWindow("#token=abcdabcdabcdabcd&fp=sha256:00");
    expect(readPairingFromLocation()).toBe("https://h:7420/#token=abcdabcdabcdabcd&fp=sha256:00");
    stripPairingFromLocation();
    expect(replaceState).toHaveBeenCalledWith(null, "", "/connect");
  });

  it("does nothing on a clean url", () => {
    const replaceState = stubWindow("");
    expect(readPairingFromLocation()).toBeNull();
    stripPairingFromLocation();
    expect(replaceState).not.toHaveBeenCalled();
  });

  it("also drops a ?url= deep link", () => {
    const replaceState = stubWindow("", "?url=https%3A%2F%2Fh");
    stripPairingFromLocation();
    expect(replaceState).toHaveBeenCalled();
  });
});
