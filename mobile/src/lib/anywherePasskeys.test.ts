import { x25519 } from "@noble/curves/ed25519.js";
import { describe, expect, it } from "vitest";

import { base64Url } from "./anywhereApi";
import {
  beginPasskeyRegistration,
  browserPasskeyExchange,
  openPasskeySecret,
  passkeyChannelAad,
  passkeyChannelKey,
  passkeyPrfWrapKey,
  sealPasskeySecret,
} from "./anywherePasskeys";
import { bytesToHex } from "./transport/anywhereEnvelope";

describe("Anywhere passkey recovery channel", () => {
  it("delivers registration entropy before opening the system browser", async () => {
    const account = "30".repeat(16);
    const sessionToken = base64Url(new Uint8Array(32).fill(0x31));
    const entropy = new Uint8Array(16).fill(0x32);
    const events: string[] = [];
    let clientPublicKey: Uint8Array | undefined;

    const result = await beginPasskeyRegistration(
      "https://app.test",
      "access-token",
      account,
      entropy,
      async () => { events.push("open"); },
      {
        createSession: async (_serviceUrl, _token, kind, publicKey) => {
          expect(kind).toBe("registration");
          clientPublicKey = publicKey;
          return {
            version: 1,
            session_token: sessionToken,
            browser_url: "https://app.test/anywhere/passkey#session=test",
            expires_at_ms: Date.now() + 60_000,
          };
        },
        sendEntropy: async (_serviceUrl, _token, sentSessionToken, ciphertext) => {
          expect(sentSessionToken).toBe(sessionToken);
          const browser = browserPasskeyExchange(sessionToken);
          const key = passkeyChannelKey(browser.privateKey, clientPublicKey!, account, sessionToken);
          expect(openPasskeySecret(ciphertext, key, passkeyChannelAad(account, "registration")))
            .toEqual(entropy);
          events.push("deliver");
        },
      },
    );

    expect(result.created.session_token).toBe(sessionToken);
    expect(events).toEqual(["deliver", "open"]);
  });

  it("keeps the browser exchange key stable when a native ceremony URL is reopened", () => {
    const session = base64Url(new Uint8Array(32).fill(0x20));
    const first = browserPasskeyExchange(session);
    const reopened = browserPasskeyExchange(session);
    const different = browserPasskeyExchange(base64Url(new Uint8Array(32).fill(0x21)));

    expect(reopened.privateKey).toEqual(first.privateKey);
    expect(reopened.publicKey).toEqual(first.publicKey);
    expect(different.privateKey).not.toEqual(first.privateKey);
  });

  it("derives the same account- and session-bound key on both devices", () => {
    const claimant = new Uint8Array(32).fill(0x21);
    const browser = new Uint8Array(32).fill(0x22);
    const account = "31".repeat(16);
    const session = base64Url(new Uint8Array(32).fill(0x32));
    const claimantKey = passkeyChannelKey(claimant, x25519.getPublicKey(browser), account, session);
    const browserKey = passkeyChannelKey(browser, x25519.getPublicKey(claimant), account, session);
    expect(claimantKey).toEqual(browserKey);
    expect(passkeyChannelKey(claimant, x25519.getPublicKey(browser), account, base64Url(new Uint8Array(32).fill(0x33))))
      .not.toEqual(claimantKey);
  });

  it("round-trips only with the authenticated transcript binding", () => {
    const key = new Uint8Array(32).fill(0x41);
    const entropy = new Uint8Array(16).fill(0x42);
    const aad = passkeyChannelAad("43".repeat(16), "authentication");
    const sealed = sealPasskeySecret(entropy, key, aad);
    expect(openPasskeySecret(sealed, key, aad)).toEqual(entropy);
    expect(() => openPasskeySecret(sealed, key, passkeyChannelAad("44".repeat(16), "authentication"))).toThrow();
  });

  it("matches the normative Rust PRF and channel vectors", () => {
    const account = "51".repeat(16);
    expect(bytesToHex(passkeyPrfWrapKey(
      new Uint8Array(32).fill(0x52),
      base64Url(new Uint8Array(32).fill(0x53)),
      account,
    ))).toBe("9eee0c00da777ada020e5b00e7ba8815137b38a87a8e1bc264dc85c923c45a36");
    expect(bytesToHex(passkeyChannelKey(
      new Uint8Array(32).fill(0x54),
      x25519.getPublicKey(new Uint8Array(32).fill(0x55)),
      account,
      base64Url(new Uint8Array(32).fill(0x56)),
    ))).toBe("e98671eb55a7ba76134fd9034b0afe92f8ee8314ae4c80fbbb747dbc63fec9b9");
  });
});
