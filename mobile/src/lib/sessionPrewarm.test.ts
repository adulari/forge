import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const replace = vi.fn();

vi.mock("expo-router", () => ({ router: { replace } }));
vi.mock("react-native", () => ({ Platform: { OS: "web" } }));
vi.mock("../theme/useBreakpoint", () => ({ useBreakpoint: () => ({ isExpanded: true }) }));

const store = await import("./sessionPrewarm");

describe("session activation", () => {
  beforeEach(() => {
    replace.mockClear();
    vi.useFakeTimers();
    vi.stubGlobal("window", {});
  });

  afterEach(() => {
    store.settleActivation(null);
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("names the session at once and lets the URL follow from an idle slice", () => {
    store.activateSession("b");
    expect(store.getActivationOverride()).toBe("b");
    expect(replace).not.toHaveBeenCalled();
    vi.advanceTimersByTime(200);
    expect(replace).toHaveBeenCalledWith("/session/b");
  });

  it("reopens a session at the tab it was left on", () => {
    store.rememberSessionPath("b", "/session/b/tasks");
    store.activateSession("b");
    vi.advanceTimersByTime(200);
    expect(replace).toHaveBeenCalledWith("/session/b/tasks");
  });

  it("only navigates to the latest of several quick clicks", () => {
    store.activateSession("b");
    store.activateSession("c");
    vi.advanceTimersByTime(300);
    expect(replace).toHaveBeenCalledTimes(1);
    expect(replace).toHaveBeenCalledWith("/session/c");
  });

  it("settles when the URL catches up, and drops the override on unrelated navigation", () => {
    store.activateSession("b");
    store.settleActivation("a"); // still the old URL while the request is pending: keep the override
    expect(store.getActivationOverride()).toBe("b");
    vi.advanceTimersByTime(200);
    store.settleActivation("b");
    expect(store.getActivationOverride()).toBeNull();

    store.activateSession("c");
    vi.advanceTimersByTime(200);
    store.settleActivation("d"); // something else navigated after the request was sent
    expect(store.getActivationOverride()).toBeNull();
  });
});
