import { describe, expect, it } from "vitest";

import {
  appendIncomingShareText,
  decodeIncomingShare,
  textFromSharedPayloads,
} from "./incomingShareCore";

describe("incoming share", () => {
  it("combines only supported non-empty text and URL payloads", () => {
    expect(textFromSharedPayloads([
      { shareType: "text", value: "  inspect this  ", mimeType: "text/plain" },
      { shareType: "url", value: "https://example.com", mimeType: "text/plain" },
      { shareType: "image", value: "file:///private/image.png", mimeType: "image/png" },
    ])).toBe("inspect this\n\nhttps://example.com");
  });

  it("rejects unsupported and oversized payloads", () => {
    expect(textFromSharedPayloads([
      { shareType: "image", value: "file:///private/image.png", mimeType: "image/png" },
    ])).toBeNull();
    expect(textFromSharedPayloads([
      { shareType: "text", value: "x".repeat(65_537), mimeType: "text/plain" },
    ])).toBeNull();
  });

  it("appends a share to an existing composer draft without dropping either", () => {
    expect(appendIncomingShareText("", "shared")).toBe("shared");
    expect(appendIncomingShareText("existing", "shared")).toBe("existing\n\nshared");
    expect(appendIncomingShareText("existing \n", "shared")).toBe("existing\n\nshared");
  });

  it("strictly decodes the durable handoff", () => {
    expect(decodeIncomingShare(JSON.stringify({
      id: "share-1",
      text: "hello",
      createdAt: 123,
    }))).toEqual({ id: "share-1", text: "hello", createdAt: 123 });
    expect(decodeIncomingShare("{bad")).toBeNull();
    expect(decodeIncomingShare(JSON.stringify({
      id: "share-1",
      text: "",
      createdAt: 123,
    }))).toBeNull();
  });
});
