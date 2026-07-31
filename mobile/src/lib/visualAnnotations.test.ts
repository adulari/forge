import { afterEach, describe, expect, it } from "vitest";

import {
  __testing,
  addVisualAnnotation,
  clearVisualAnnotations,
  formatVisualAnnotationsPrompt,
  getVisualAnnotations,
  removeVisualAnnotation,
  visualAnnotationLabel,
} from "./visualAnnotations";

const picked = {
  url: "http://localhost:5173/settings",
  title: "Settings",
  selector: "main > form > button:nth-of-type(2)",
  tagName: "button",
  elementId: null,
  role: "button",
  accessibleName: "Save profile",
  text: "Save",
  attributes: [{ name: "data-testid", value: "save-profile" }],
  rect: { x: 40, y: 320, width: 120, height: 36 },
};

afterEach(() => __testing.reset());

describe("visual annotation store", () => {
  it("scopes picked elements to their session and removes them explicitly", () => {
    const first = addVisualAnnotation("session-a", picked);
    addVisualAnnotation("session-b", { ...picked, selector: "#other" });
    expect(getVisualAnnotations("session-a")).toHaveLength(1);
    expect(getVisualAnnotations("session-b")).toHaveLength(1);

    removeVisualAnnotation("session-a", first.id);
    expect(getVisualAnnotations("session-a")).toEqual([]);
    expect(getVisualAnnotations("session-b")).toHaveLength(1);

    clearVisualAnnotations("session-b");
    expect(getVisualAnnotations("session-b")).toEqual([]);
  });
});

it("uses accessible names for compact composer labels", () => {
  expect(visualAnnotationLabel(picked)).toBe("Save profile");
});

it("formats exact selector, page, attributes, bounds, and visible text for the agent", () => {
  const annotation = addVisualAnnotation("session-a", picked);
  const prompt = formatVisualAnnotationsPrompt("Make this action clearer.", [annotation]);
  expect(prompt).toContain("Make this action clearer.");
  expect(prompt).toContain("`main > form > button:nth-of-type(2)`");
  expect(prompt).toContain("http://localhost:5173/settings");
  expect(prompt).toContain('accessible name="Save profile"');
  expect(prompt).toContain('data-testid="save-profile"');
  expect(prompt).toContain("120×36 at 40,320 CSS px");
  expect(prompt).toContain('Visible text: "Save"');
});

it("supplies an actionable default when annotations are sent without typed text", () => {
  const annotation = addVisualAnnotation("session-a", picked);
  expect(formatVisualAnnotationsPrompt("", [annotation])).toMatch(
    /^Please update the selected interface elements\./,
  );
});
