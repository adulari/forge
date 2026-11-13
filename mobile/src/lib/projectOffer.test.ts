import { describe, expect, it } from "vitest";

import { projectOfferCopy, projectOfferDismiss, projectOfferMove } from "./projectOffer";

const offer = { path: "/home/u/code/forge", name: "forge", calls: 3 };

describe("project offer", () => {
  it("echoes the offered path in both answers", () => {
    expect(projectOfferMove(offer)).toEqual({ kind: "project_move", path: "/home/u/code/forge" });
    expect(projectOfferDismiss(offer)).toEqual({ kind: "project_dismiss", path: "/home/u/code/forge" });
  });

  it("names the repo in the title and says what moving gives", () => {
    const copy = projectOfferCopy(offer);
    expect(copy.title).toBe("This session is working in forge. Move it there?");
    expect(copy.detail).toContain("Lattice");
  });
});
