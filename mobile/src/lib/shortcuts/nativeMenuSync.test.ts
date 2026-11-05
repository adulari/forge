import { describe, expect, it, vi } from "vitest";

import { getResolvedShortcut, setAppShortcut } from "./bindings";

const storage = vi.hoisted(() => ({
  getItem: vi.fn(async () => null as string | null),
  setItem: vi.fn(async () => undefined),
}));
const tauri = vi.hoisted(() => ({
  invoke: vi.fn(async () => {
    throw new Error("menu accelerator is not mutable: view:browser-preview");
  }),
}));

vi.mock("@react-native-async-storage/async-storage", () => ({ default: storage }));
vi.mock("../platform", () => ({ isTauri: true }));
vi.mock("@tauri-apps/api/core", () => tauri);

describe("native menu sync failure", () => {
  it("keeps the saved edit when the native menu rejects", async () => {
    vi.stubGlobal("__DEV__", false);
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const next = { key: "9", mod: false, alt: true, shift: false };
    await expect(setAppShortcut("nav.fleet", next)).resolves.toBeUndefined();
    expect(getResolvedShortcut("nav.fleet")).toEqual(next);
    expect(tauri.invoke).toHaveBeenCalledWith("set_menu_accelerators", expect.anything());
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });
});
