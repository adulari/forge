import { describe, expect, it } from "vitest";

import { parseConnectUrl } from "./connectUrl";

describe("Forge server URLs", () => {
  it("normalizes connect links and preserves token-scoped paths", () => {
    expect(parseConnectUrl("connect://forge.example/aabbccddeeff0011")).toEqual({
      baseUrl: "https://forge.example/aabbccddeeff0011",
      token: "aabbccddeeff0011",
      host: "forge.example",
    });
    expect(parseConnectUrl(" http://127.0.0.1:7452/aabbccddeeff0011/ ")?.baseUrl).toBe(
      "http://127.0.0.1:7452/aabbccddeeff0011",
    );
  });

  it("rejects unsafe schemes and malformed credentials", () => {
    expect(parseConnectUrl("javascript://forge/aabbccddeeff0011")).toBeNull();
    expect(parseConnectUrl("https://forge.example/not-a-token")).toBeNull();
  });
});
