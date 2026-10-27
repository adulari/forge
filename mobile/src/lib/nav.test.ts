import { beforeEach, describe, expect, it, vi } from "vitest";

const push = vi.fn();
const replace = vi.fn();

vi.mock("expo-router", () => ({ router: { push, replace, canGoBack: () => false } }));
vi.mock("react-native", () => ({ Platform: { OS: "web" } }));

const { openSession, rememberShellRoute } = await import("./nav");

function setWidth(width: number): void {
  vi.stubGlobal("window", { innerWidth: width });
}

describe("openSession", () => {
  beforeEach(() => {
    push.mockClear();
    replace.mockClear();
  });

  it("replaces the current session on a wide desktop window, so visited sessions do not pile up mounted", () => {
    setWidth(1440);
    rememberShellRoute("/session/a/tasks");
    openSession("b");
    expect(replace).toHaveBeenCalledWith("/session/b");
    expect(push).not.toHaveBeenCalled();
  });

  it("pushes when coming from a non-session route", () => {
    setWidth(1440);
    rememberShellRoute("/");
    openSession("b");
    expect(push).toHaveBeenCalledWith("/session/b");
    expect(replace).not.toHaveBeenCalled();
  });

  it("pushes on a narrow window, where Back returns to the previous session", () => {
    setWidth(700);
    rememberShellRoute("/session/a");
    openSession("b");
    expect(push).toHaveBeenCalledWith("/session/b");
    expect(replace).not.toHaveBeenCalled();
  });

  it("does nothing when the session is already open", () => {
    setWidth(1440);
    rememberShellRoute("/session/a");
    openSession("a");
    expect(push).not.toHaveBeenCalled();
    expect(replace).not.toHaveBeenCalled();
  });
});
