import { describe, expect, it } from "vitest";

import { downloadName } from "./imageTypes";

describe("downloadName", () => {
  it("keeps a name that already has an image extension", () => {
    expect(downloadName({ uri: "u", name: "shot.PNG" })).toBe("shot.PNG");
  });

  it("adds the extension for the image type and cleans unsafe characters", () => {
    expect(downloadName({ uri: "u", name: "a/b:c" }, "image/jpeg")).toBe("a_b_c.jpg");
    expect(downloadName({ uri: "u", name: "" })).toBe("image.png");
  });
});
