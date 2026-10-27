import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const push = vi.fn();
const replace = vi.fn();

vi.mock("expo-router", () => ({ router: { push, replace, canGoBack: () => false } }));
vi.mock("react-native", () => ({ Platform: { OS: "web" } }));
vi.mock("../theme/useBreakpoint", () => ({ useBreakpoint: () => ({ isExpanded: true }) }));

const { openSession, rememberShellRoute } = await import("./nav");
const { getActivationOverride, settleActivation } = await import("./sessionPrewarm");

function setWidth(width: number): void {
  vi.stubGlobal("window", { innerWidth: width });
}

describe("openSession", () => {
  beforeEach(() => {
    push.mockClear();
    replace.mockClear();
    vi.useFakeTimers();
  });

  afterEach(() => {
    settleActivation(null);
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("activates the session locally on a wide desktop window and lets the URL follow", () => {
    setWidth(1440);
    rememberShellRoute("/session/a/tasks");
    openSession("b");
    expect(getActivationOverride()).toBe("b");
    expect(push).not.toHaveBeenCalled();
    vi.advanceTimersByTime(200);
    expect(replace).toHaveBeenCalledWith("/session/b");
    expect(push).not.toHaveBeenCalled();
  });

  it("pushes when coming from a non-session route", () => {
    setWidth(1440);
    rememberShellRoute("/");
    openSession("b");
    expect(push).toHaveBeenCalledWith("/session/b");
    expect(getActivationOverride()).toBeNull();
  });

  it("pushes on a narrow window, where Back returns to the previous session", () => {
    setWidth(700);
    rememberShellRoute("/session/a");
    openSession("b");
    expect(push).toHaveBeenCalledWith("/session/b");
    expect(getActivationOverride()).toBeNull();
  });

  it("does nothing when the session is already open", () => {
    setWidth(1440);
    rememberShellRoute("/session/a");
    openSession("a");
    vi.advanceTimersByTime(200);
    expect(push).not.toHaveBeenCalled();
    expect(replace).not.toHaveBeenCalled();
    expect(getActivationOverride()).toBeNull();
  });
});
