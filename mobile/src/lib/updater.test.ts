import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  checkDesktopUpdate,
  desktopUpdateAction,
  getDesktopUpdateState,
  installDesktopUpdate,
  type DesktopUpdateState,
} from "./updater";

const mocks = vi.hoisted(() => ({
  check: vi.fn(),
  downloadAndInstall: vi.fn(),
  relaunch: vi.fn(),
  invoke: vi.fn(),
}));

vi.mock("./platform", () => ({ isTauri: true }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: mocks.check }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: mocks.relaunch }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

describe("desktop updater coordination", () => {
  beforeEach(() => {
    mocks.check.mockReset();
    mocks.downloadAndInstall.mockReset();
    mocks.relaunch.mockReset();
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue(true);
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

  it("offers the release page instead of installing over a non-AppImage Linux install", async () => {
    mocks.invoke.mockResolvedValue(false);
    mocks.check.mockResolvedValue({
      version: "3.1.0",
      body: null,
      downloadAndInstall: mocks.downloadAndInstall,
    });

    await checkDesktopUpdate();

    expect(mocks.invoke).toHaveBeenCalledWith("desktop_update_installable");
    expect(getDesktopUpdateState()).toMatchObject({
      phase: "available",
      availableVersion: "3.1.0",
      selfInstall: false,
      releaseUrl: "https://github.com/Adulari/forge/releases/tag/v3.1.0",
    });
    expect(desktopUpdateAction(getDesktopUpdateState())).toBe("release-page");
    await expect(installDesktopUpdate()).rejects.toThrow(/release page/);
    expect(mocks.downloadAndInstall).not.toHaveBeenCalled();
    expect(mocks.relaunch).not.toHaveBeenCalled();
  });

  it("treats an unanswered install probe as not installable", async () => {
    mocks.invoke.mockRejectedValue(new Error("command not found"));
    mocks.check.mockResolvedValue({ version: "3.2.0", downloadAndInstall: mocks.downloadAndInstall });

    await checkDesktopUpdate();

    expect(getDesktopUpdateState().selfInstall).toBe(false);
    await expect(installDesktopUpdate()).rejects.toThrow();
    expect(mocks.downloadAndInstall).not.toHaveBeenCalled();
  });
});

describe("desktop update panel action", () => {
  const base: DesktopUpdateState = {
    phase: "available",
    checkedAt: 1,
    availableVersion: "3.0.0",
    body: null,
    message: null,
    selfInstall: true,
    releaseUrl: "https://github.com/Adulari/forge/releases/tag/v3.0.0",
  };

  it("installs only when the install can replace itself", () => {
    expect(desktopUpdateAction(base)).toBe("install");
    expect(desktopUpdateAction({ ...base, selfInstall: false })).toBe("release-page");
  });

  it("checks again in every other phase", () => {
    expect(desktopUpdateAction({ ...base, phase: "installing" })).toBe("installing");
    for (const phase of ["idle", "checking", "up-to-date", "error"] as const) {
      expect(desktopUpdateAction({ ...base, phase, selfInstall: false })).toBe("check");
    }
  });
});
