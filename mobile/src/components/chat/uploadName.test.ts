import { describe, expect, it } from "vitest";

import { uploadDisplayName } from "./uploadName";

describe("uploadDisplayName", () => {
  it("drops the timestamp and nonce the daemon adds", () => {
    expect(
      uploadDisplayName("/home/u/.forge/uploads/s1/1789785919917-3f1c3ddda03c0938-forge-attach-test.txt"),
    ).toBe("forge-attach-test.txt");
  });

  it("still reads uploads from builds without the nonce", () => {
    expect(uploadDisplayName("/tmp/.forge/uploads/s1/1699999999999-notes.txt")).toBe("notes.txt");
  });
});
