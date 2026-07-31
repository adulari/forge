import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  checkDesktopUpdate,
  getDesktopUpdateState,
  installDesktopUpdate,
} from "./updater";

const mocks = vi.hoisted(() => ({
  check: vi.fn(),
  downloadAndInstall: vi.fn(),
  relaunch: vi.fn(),
}));

vi.mock("./platform", () => ({ isTauri: true }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: mocks.check }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: mocks.relaunch }));

describe("desktop updater coordination", () => {
  beforeEach(() => {
    mocks.check.mockReset();
    mocks.downloadAndInstall.mockReset();
    mocks.relaunch.mockReset();
  });

  it("publishes one discovered update and installs that exact signed artifact", async () => {
    mocks.check.mockResolvedValue({
      version: "3.0.0",
      body: "Release notes",
      downloadAndInstall: mocks.downloadAndInstall,
    });
    mocks.downloadAndInstall.mockResolvedValue(undefined);
    mocks.relaunch.mockResolvedValue(undefined);

    const first = checkDesktopUpdate();
    const second = checkDesktopUpdate();
    expect(getDesktopUpdateState().phase).toBe("checking");
    await Promise.all([first, second]);

    expect(mocks.check).toHaveBeenCalledTimes(1);
    expect(getDesktopUpdateState()).toMatchObject({
      phase: "available",
      availableVersion: "3.0.0",
      body: "Release notes",
      message: null,
    });

    await installDesktopUpdate();
    expect(mocks.downloadAndInstall).toHaveBeenCalledTimes(1);
    expect(mocks.relaunch).toHaveBeenCalledTimes(1);
  });
});
