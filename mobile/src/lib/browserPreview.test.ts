import { afterEach, describe, expect, it, vi } from "vitest";

import {
  __testing,
  getPreviewPreferences,
  normalizePreviewUrl,
  previewLabel,
  previewViewportBounds,
  setPreviewPreferences,
} from "./browserPreview";

vi.mock("./platform", () => ({ isTauri: false }));

afterEach(() => __testing.clearPreferences());

describe("normalizePreviewUrl", () => {
  it("turns a bare local port into a localhost HTTP URL", () => {
    expect(normalizePreviewUrl("5173")).toBe("http://localhost:5173/");
  });

  it("uses HTTP for loopback and HTTPS for public hostnames", () => {
    expect(normalizePreviewUrl("localhost:3000/app")).toBe("http://localhost:3000/app");
    expect(normalizePreviewUrl("127.0.0.1:8080")).toBe("http://127.0.0.1:8080/");
    expect(normalizePreviewUrl("example.com/docs")).toBe("https://example.com/docs");
  });

  it("rejects non-HTTP protocols", () => {
    expect(() => normalizePreviewUrl("file:///etc/passwd")).toThrow(/HTTP or HTTPS/);
    expect(() => normalizePreviewUrl("javascript:alert(1)")).toThrow(/HTTP or HTTPS/);
  });
});

it("creates stable capability-safe labels without exposing the surface id", () => {
  const id = "preview:session/private workspace:http://localhost:5173";
  expect(previewLabel(id)).toMatch(/^preview-[a-f0-9]{8}-[a-f0-9]{8}$/);
  expect(previewLabel(id)).toBe(previewLabel(id));
  expect(previewLabel(id)).not.toContain("private");
  expect(previewLabel(`${id}/other`)).not.toBe(previewLabel(id));
});

it("centres device widths and never exceeds the measured panel", () => {
  expect(
    previewViewportBounds({ x: 100, y: 50, width: 900, height: 600 }, "mobile"),
  ).toEqual({ x: 355, y: 50, width: 390, height: 600 });
  expect(
    previewViewportBounds({ x: 10, y: 20, width: 320, height: 500 }, "tablet"),
  ).toEqual({ x: 10, y: 20, width: 320, height: 500 });
});

it("retains per-preview URL, zoom, and viewport choices", () => {
  expect(getPreviewPreferences("preview-a")).toEqual({ url: "", zoom: 1, viewport: "fill" });
  setPreviewPreferences("preview-a", { url: "http://localhost:5173/", viewport: "mobile" });
  setPreviewPreferences("preview-a", { zoom: 1.25 });
  expect(getPreviewPreferences("preview-a")).toEqual({
    url: "http://localhost:5173/",
    zoom: 1.25,
    viewport: "mobile",
  });
});
