import { describe, expect, it } from "vitest";

import { composeManualLink, describeConnectTarget, normalizeFingerprint, parseConnectUrl } from "./connectUrl";

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

describe("pairing link with a pinned certificate", () => {
  const token = "a".repeat(32);
  const hex = "ab".repeat(32);

  it("parses the fragment form and keeps the pin", () => {
    const parsed = parseConnectUrl(`https://192.168.1.5:7420/#token=${token}&fp=sha256:${hex}`);
    expect(parsed).toEqual({
      baseUrl: `https://192.168.1.5:7420/${token}`,
      token,
      host: "192.168.1.5:7420",
      fingerprint: `sha256:${hex}`,
    });
  });

  it("still parses the path form, without a pin", () => {
    const parsed = parseConnectUrl(`https://box.ts.net/${token}`);
    expect(parsed?.fingerprint).toBeUndefined();
    expect(parsed?.baseUrl).toBe(`https://box.ts.net/${token}`);
  });

  it("accepts colon-separated uppercase fingerprints", () => {
    const colon = hex.toUpperCase().match(/../g)!.join(":");
    expect(normalizeFingerprint(colon)).toBe(`sha256:${hex}`);
  });

  it("rejects a malformed pin instead of silently dropping it", () => {
    expect(parseConnectUrl(`https://h:7420/#token=${token}&fp=sha256:zz`)).toBeNull();
    expect(parseConnectUrl(`https://h:7420/#token=short`)).toBeNull();
  });

  it("describes the target from the parsed host", () => {
    const plain = parseConnectUrl(`http://192.168.1.5:7420/${token}`)!;
    expect(describeConnectTarget(plain, true)).toMatchObject({ cleartext: true, private: true, lookalike: false, pinned: false });
    const lookalike = parseConnectUrl(`https://xn--forge-9ua.example/${token}`)!;
    expect(describeConnectTarget(lookalike, true).lookalike).toBe(true);
    const pinned = parseConnectUrl(`https://10.0.0.2:7420/#token=${token}&fp=sha256:${hex}`)!;
    expect(describeConnectTarget(pinned, true)).toMatchObject({ pinned: true, pinIgnored: false });
  });

  it("claims a pin only where it is enforced: https on a platform that checks it", () => {
    const withPin = (scheme: string) => parseConnectUrl(`${scheme}://10.0.0.2:7420/#token=${token}&fp=sha256:${hex}`)!;
    expect(describeConnectTarget(withPin("https"), false)).toMatchObject({ pinned: false, pinIgnored: true });
    expect(describeConnectTarget(withPin("http"), true)).toMatchObject({ pinned: false, pinIgnored: true, cleartext: true });
    expect(describeConnectTarget(parseConnectUrl(`https://10.0.0.2:7420/${token}`)!, true)).toMatchObject({
      pinned: false,
      pinIgnored: false,
    });
  });
});

describe("manual pairing link", () => {
  const token = "c".repeat(32);

  it("builds a link the parser accepts, with a fingerprint the user typed", () => {
    const fp = `sha256:${"ef".repeat(32)}`;
    const link = composeManualLink(" https://192.168.1.20/ ", "7420", token, fp);
    expect(parseConnectUrl(link ?? "")).toMatchObject({ host: "192.168.1.20:7420", token, fingerprint: fp });
  });

  it("normalises a typed fingerprint and refuses a malformed one", () => {
    const colon = "AB".repeat(32).match(/../g)!.join(":");
    expect(parseConnectUrl(composeManualLink("h", "7420", token, colon) ?? "")?.fingerprint).toBe(`sha256:${"ab".repeat(32)}`);
    expect(composeManualLink("h", "7420", token, "sha256:nope")).toBeNull();
    expect(parseConnectUrl(composeManualLink("h", "7420", token, "  ") ?? "")?.fingerprint).toBeUndefined();
  });

  it("needs a host and a token", () => {
    expect(composeManualLink("", "7420", token)).toBeNull();
    expect(composeManualLink("host", "7420", " ")).toBeNull();
  });
});
