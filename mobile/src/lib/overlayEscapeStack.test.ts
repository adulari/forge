import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { openOverlayCount, registerOverlay } from "./overlayEscapeStack";

type Listener = (event: unknown) => void;

function press(fire: (event: unknown) => void, init: Record<string, unknown> = {}) {
  const event = {
    key: "Escape",
    defaultPrevented: false,
    isComposing: false,
    keyCode: 27,
    target: null,
    preventDefault: vi.fn(),
    stopPropagation: vi.fn(),
    ...init,
  };
  fire(event);
  return event;
}

describe("overlayEscapeStack", () => {
  let listeners: Listener[];
  let fire: (event: unknown) => void;
  const disposers: (() => void)[] = [];

  beforeEach(() => {
    listeners = [];
    fire = (event) => listeners.forEach((listener) => listener(event));
    vi.stubGlobal("window", {
      addEventListener: (_type: string, listener: Listener) => listeners.push(listener),
      removeEventListener: (_type: string, listener: Listener) => {
        listeners = listeners.filter((l) => l !== listener);
      },
    });
  });

  afterEach(() => {
    while (disposers.length) disposers.pop()?.();
    vi.unstubAllGlobals();
  });

  const register = (close: () => void) => {
    const dispose = registerOverlay(close);
    disposers.push(dispose);
    return dispose;
  };

  it("closes only the topmost overlay and swallows the key", () => {
    const sheet = vi.fn();
    const palette = vi.fn();
    register(sheet);
    register(palette);
    const event = press(fire);
    expect(palette).toHaveBeenCalledTimes(1);
    expect(sheet).not.toHaveBeenCalled();
    expect(event.stopPropagation).toHaveBeenCalled();
    expect(event.preventDefault).toHaveBeenCalled();
  });

  it("reveals the overlay underneath once the top one is gone", () => {
    const sheet = vi.fn();
    const disposePalette = (register(sheet), register(vi.fn()));
    disposePalette();
    press(fire);
    expect(sheet).toHaveBeenCalledTimes(1);
  });

  it("leaves IME composition alone", () => {
    const close = vi.fn();
    register(close);
    const event = press(fire, { isComposing: true });
    expect(close).not.toHaveBeenCalled();
    expect(event.stopPropagation).not.toHaveBeenCalled();
    press(fire, { keyCode: 229 });
    expect(close).not.toHaveBeenCalled();
  });

  it("leaves the terminal's Escape alone", () => {
    const close = vi.fn();
    register(close);
    press(fire, { target: { closest: (selector: string) => (selector === "[data-escape-skip]" ? {} : null) } });
    expect(close).not.toHaveBeenCalled();
    press(fire, { target: { closest: () => null } });
    expect(close).toHaveBeenCalledTimes(1);
  });

  it("ignores other keys and already-handled events", () => {
    const close = vi.fn();
    register(close);
    press(fire, { key: "Enter" });
    press(fire, { defaultPrevented: true });
    expect(close).not.toHaveBeenCalled();
  });

  it("removes the window listener when the stack empties", () => {
    const dispose = register(vi.fn());
    expect(listeners).toHaveLength(1);
    expect(openOverlayCount()).toBe(1);
    dispose();
    expect(listeners).toHaveLength(0);
    expect(openOverlayCount()).toBe(0);
  });
});
