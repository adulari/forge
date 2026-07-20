import { xchacha20poly1305 } from "@noble/ciphers/chacha.js";
import { x25519 } from "@noble/curves/ed25519.js";
import { hkdf } from "@noble/hashes/hkdf.js";
import { sha256 } from "@noble/hashes/sha2.js";

import { anywhereRequest, base64Url, fromBase64Url } from "./anywhereApi";
import { secureRandomBytes } from "./secureRandom";
import { bytesFromHex } from "./transport/anywhereEnvelope";

const CHANNEL_CONTEXT = new TextEncoder().encode("forge-anywhere/v2/passkey-channel");
const WRAP_CONTEXT = new TextEncoder().encode("forge-anywhere/v2/passkey-prf-wrap");

export interface AnywherePasskey {
  id: string;
  name: string;
  created_at: number;
  last_used_at: number | null;
}

export interface PasskeySession {
  version: 1;
  kind: "registration" | "authentication";
  account_id: string;
  client_exchange_public_key: string;
  browser_exchange_public_key: string | null;
  to_browser_ciphertext: string | null;
  to_client_ciphertext: string | null;
  expires_at_ms: number;
  completed: boolean;
}

interface CreatedSession {
  version: 1;
  session_token: string;
  browser_url: string;
  expires_at_ms: number;
}

interface OptionsResponse {
  version: 1;
  public_key: { publicKey: Record<string, unknown> };
  prf_salt: string;
}

export function generatePasskeyExchange(): { privateKey: Uint8Array; publicKey: Uint8Array } {
  const privateKey = secureRandomBytes(32);
  return { privateKey, publicKey: x25519.getPublicKey(privateKey) };
}

export async function listPasskeys(serviceUrl: string, token: string): Promise<AnywherePasskey[]> {
  const response = await anywhereRequest<{ version: 1; passkeys: AnywherePasskey[] }>(
    serviceUrl,
    "/v1/passkeys",
    { cache: "no-store" },
    token,
  );
  if (response.version !== 1 || !Array.isArray(response.passkeys)) {
    throw new Error("Forge Anywhere returned an invalid passkey list.");
  }
  return response.passkeys;
}

export async function renamePasskey(serviceUrl: string, token: string, id: string, name: string): Promise<void> {
  await anywhereRequest(serviceUrl, `/v1/passkeys/${id}`, {
    method: "PATCH",
    body: JSON.stringify({ version: 1, name }),
  }, token);
}

export async function revokePasskey(serviceUrl: string, token: string, id: string): Promise<void> {
  await anywhereRequest(serviceUrl, `/v1/passkeys/${id}`, { method: "DELETE" }, token);
}

export async function createPasskeySession(
  serviceUrl: string,
  token: string,
  kind: "registration" | "authentication",
  clientPublicKey: Uint8Array,
  passkeyId?: string,
): Promise<CreatedSession> {
  return anywhereRequest<CreatedSession>(serviceUrl, `/v1/passkey-sessions/${kind}`, {
    method: "POST",
    body: JSON.stringify({
      version: 1,
      client_exchange_public_key: base64Url(clientPublicKey),
      ...(passkeyId ? { passkey_id: passkeyId } : {}),
    }),
  }, token);
}

export async function getPasskeySession(serviceUrl: string, sessionToken: string): Promise<PasskeySession> {
  return anywhereRequest<PasskeySession>(serviceUrl, "/v1/passkey-sessions/current", {
    cache: "no-store",
    headers: sessionHeaders(sessionToken),
  });
}

export async function sendRegistrationEntropy(
  serviceUrl: string,
  token: string,
  sessionToken: string,
  ciphertext: string,
): Promise<void> {
  await anywhereRequest(serviceUrl, "/v1/passkey-sessions/payload", {
    method: "POST",
    headers: sessionHeaders(sessionToken),
    body: JSON.stringify({ version: 1, ciphertext }),
  }, token);
}

export function passkeyChannelKey(
  privateKey: Uint8Array,
  publicKey: Uint8Array,
  accountIdHex: string,
  sessionToken: string,
): Uint8Array {
  const shared = x25519.getSharedSecret(privateKey, publicKey);
  return hkdf(
    sha256,
    shared,
    bytesFromHex(accountIdHex),
    concat(CHANNEL_CONTEXT, sha256(fromBase64Url(sessionToken))),
    32,
  );
}

export function sealPasskeySecret(secret: Uint8Array, key: Uint8Array, aad: string): string {
  const nonce = secureRandomBytes(24);
  const encrypted = xchacha20poly1305(key, nonce, new TextEncoder().encode(aad)).encrypt(secret);
  return base64Url(concat(nonce, encrypted));
}

export function openPasskeySecret(encoded: string, key: Uint8Array, aad: string): Uint8Array {
  const blob = fromBase64Url(encoded);
  if (blob.length < 41) throw new Error("Encrypted passkey recovery data is incomplete.");
  return xchacha20poly1305(
    key,
    blob.slice(0, 24),
    new TextEncoder().encode(aad),
  ).decrypt(blob.slice(24));
}

export async function completeBrowserPasskeySession(
  serviceUrl: string,
  sessionToken: string,
  onStatus?: (status: string) => void,
): Promise<"registered" | "recovered"> {
  if (typeof navigator === "undefined" || !navigator.credentials || !window.PublicKeyCredential) {
    throw new Error("This browser does not support passkeys.");
  }
  const exchange = generatePasskeyExchange();
  let session = await getPasskeySession(serviceUrl, sessionToken);
  await anywhereRequest(serviceUrl, "/v1/passkey-sessions/browser-key", {
    method: "POST",
    headers: sessionHeaders(sessionToken),
    body: JSON.stringify({ version: 1, browser_exchange_public_key: base64Url(exchange.publicKey) }),
  });
  const channelKey = passkeyChannelKey(
    exchange.privateKey,
    fromBase64Url(session.client_exchange_public_key),
    session.account_id,
    sessionToken,
  );
  if (session.kind === "registration") {
    onStatus?.("Waiting for the enrolled device…");
    session = await pollSession(serviceUrl, sessionToken, (value) => Boolean(value.to_browser_ciphertext));
    const entropy = openPasskeySecret(
      session.to_browser_ciphertext!,
      channelKey,
      passkeyChannelAad(session.account_id, "registration"),
    );
    onStatus?.("Create your passkey…");
    const options = await ceremonyOptions(serviceUrl, sessionToken, "registration");
    const { credential, prfOutput } = await createCredential(options);
    const wrapKey = passkeyPrfWrapKey(prfOutput, options.prf_salt, session.account_id);
    const recoveryWrap = sealPasskeySecret(entropy, wrapKey, wrapAad(session.account_id));
    await anywhereRequest(serviceUrl, "/v1/passkey-sessions/registration/complete", {
      method: "POST",
      headers: sessionHeaders(sessionToken),
      body: JSON.stringify({
        version: 1,
        name: browserPasskeyName(),
        credential,
        recovery_secret_wrap: recoveryWrap,
      }),
    });
    entropy.fill(0);
    wrapKey.fill(0);
    return "registered";
  }

  onStatus?.("Verify your passkey…");
  const options = await ceremonyOptions(serviceUrl, sessionToken, "authentication");
  const { credential, prfOutput } = await getCredential(options);
  const completed = await anywhereRequest<{ version: 1; recovery_secret_wrap: string }>(
    serviceUrl,
    "/v1/passkey-sessions/authentication/complete",
    {
      method: "POST",
      headers: sessionHeaders(sessionToken),
      body: JSON.stringify({ version: 1, credential }),
    },
  );
  const wrapKey = passkeyPrfWrapKey(prfOutput, options.prf_salt, session.account_id);
  const entropy = openPasskeySecret(completed.recovery_secret_wrap, wrapKey, wrapAad(session.account_id));
  await anywhereRequest(serviceUrl, "/v1/passkey-sessions/result", {
    method: "POST",
    headers: sessionHeaders(sessionToken),
    body: JSON.stringify({
      version: 1,
      ciphertext: sealPasskeySecret(
        entropy,
        channelKey,
        passkeyChannelAad(session.account_id, "authentication"),
      ),
    }),
  });
  entropy.fill(0);
  wrapKey.fill(0);
  return "recovered";
}

async function ceremonyOptions(
  serviceUrl: string,
  sessionToken: string,
  kind: "registration" | "authentication",
): Promise<OptionsResponse> {
  return anywhereRequest<OptionsResponse>(serviceUrl, `/v1/passkey-sessions/${kind}/options`, {
    method: "POST",
    headers: sessionHeaders(sessionToken),
  });
}

async function createCredential(options: OptionsResponse): Promise<{ credential: unknown; prfOutput: Uint8Array }> {
  const publicKey = decodeCreationOptions(options.public_key.publicKey, options.prf_salt);
  const credential = await navigator.credentials.create({ publicKey }) as PublicKeyCredential | null;
  if (!credential) throw new Error("Passkey creation was canceled.");
  return { credential: credentialToJson(credential), prfOutput: requiredPrfOutput(credential) };
}

async function getCredential(options: OptionsResponse): Promise<{ credential: unknown; prfOutput: Uint8Array }> {
  const publicKey = decodeRequestOptions(options.public_key.publicKey, options.prf_salt);
  const credential = await navigator.credentials.get({ publicKey }) as PublicKeyCredential | null;
  if (!credential) throw new Error("Passkey verification was canceled.");
  return { credential: credentialToJson(credential), prfOutput: requiredPrfOutput(credential) };
}

function decodeCreationOptions(value: Record<string, unknown>, salt: string): PublicKeyCredentialCreationOptions {
  const options = structuredClone(value) as unknown as PublicKeyCredentialCreationOptions;
  options.challenge = arrayBuffer(fromBase64Url(String(value.challenge)));
  options.user = { ...(value.user as PublicKeyCredentialUserEntity), id: arrayBuffer(fromBase64Url(String((value.user as { id: string }).id))) };
  options.excludeCredentials = decodeDescriptors(value.excludeCredentials);
  options.extensions = { ...(options.extensions ?? {}), prf: { eval: { first: arrayBuffer(fromBase64Url(salt)) } } } as AuthenticationExtensionsClientInputs;
  return options;
}

function decodeRequestOptions(value: Record<string, unknown>, salt: string): PublicKeyCredentialRequestOptions {
  const options = structuredClone(value) as unknown as PublicKeyCredentialRequestOptions;
  options.challenge = arrayBuffer(fromBase64Url(String(value.challenge)));
  options.allowCredentials = decodeDescriptors(value.allowCredentials);
  options.extensions = { ...(options.extensions ?? {}), prf: { eval: { first: arrayBuffer(fromBase64Url(salt)) } } } as AuthenticationExtensionsClientInputs;
  return options;
}

function decodeDescriptors(value: unknown): PublicKeyCredentialDescriptor[] | undefined {
  if (!Array.isArray(value)) return undefined;
  return value.map((descriptor) => ({ ...descriptor, id: arrayBuffer(fromBase64Url(String(descriptor.id))) }));
}

function credentialToJson(credential: PublicKeyCredential): unknown {
  const serializable = credential as PublicKeyCredential & { toJSON?(): unknown };
  if (serializable.toJSON) return serializable.toJSON();
  const response = credential.response;
  if (response instanceof AuthenticatorAttestationResponse) {
    return {
      id: credential.id,
      rawId: base64Url(new Uint8Array(credential.rawId)),
      type: credential.type,
      response: {
        clientDataJSON: base64Url(new Uint8Array(response.clientDataJSON)),
        attestationObject: base64Url(new Uint8Array(response.attestationObject)),
        transports: response.getTransports?.() ?? [],
      },
      clientExtensionResults: credential.getClientExtensionResults(),
    };
  }
  const assertion = response as AuthenticatorAssertionResponse;
  return {
    id: credential.id,
    rawId: base64Url(new Uint8Array(credential.rawId)),
    type: credential.type,
    response: {
      authenticatorData: base64Url(new Uint8Array(assertion.authenticatorData)),
      clientDataJSON: base64Url(new Uint8Array(assertion.clientDataJSON)),
      signature: base64Url(new Uint8Array(assertion.signature)),
      userHandle: assertion.userHandle ? base64Url(new Uint8Array(assertion.userHandle)) : null,
    },
    clientExtensionResults: credential.getClientExtensionResults(),
  };
}

function requiredPrfOutput(credential: PublicKeyCredential): Uint8Array {
  const extensions = credential.getClientExtensionResults() as AuthenticationExtensionsClientOutputs & {
    prf?: { results?: { first?: ArrayBuffer } };
  };
  const first = extensions.prf?.results?.first;
  if (!first) throw new Error("This passkey does not support encrypted recovery. Try another passkey provider.");
  return new Uint8Array(first);
}

export function passkeyPrfWrapKey(prfOutput: Uint8Array, encodedSalt: string, accountIdHex: string): Uint8Array {
  return hkdf(
    sha256,
    prfOutput,
    bytesFromHex(accountIdHex),
    concat(WRAP_CONTEXT, fromBase64Url(encodedSalt)),
    32,
  );
}

async function pollSession(
  serviceUrl: string,
  token: string,
  done: (session: PasskeySession) => boolean,
): Promise<PasskeySession> {
  for (;;) {
    const session = await getPasskeySession(serviceUrl, token);
    if (done(session)) return session;
    if (Date.now() >= session.expires_at_ms) throw new Error("Passkey recovery request expired.");
    await new Promise((resolve) => setTimeout(resolve, 1_500));
  }
}

function sessionHeaders(token: string): Record<string, string> {
  return { "X-Forge-Passkey-Session": token };
}

export function passkeyChannelAad(accountId: string, kind: "registration" | "authentication"): string {
  return `forge-anywhere/v2/passkey-channel:${accountId}:${kind}`;
}

function wrapAad(accountId: string): string {
  return `forge-anywhere/v2/passkey-prf-wrap:${accountId}`;
}

function browserPasskeyName(): string {
  const platform = typeof navigator === "undefined" ? "Browser" : navigator.platform || "Browser";
  return `${platform} passkey`.slice(0, 100);
}

function concat(...values: Uint8Array[]): Uint8Array {
  const output = new Uint8Array(values.reduce((sum, value) => sum + value.length, 0));
  let offset = 0;
  for (const value of values) { output.set(value, offset); offset += value.length; }
  return output;
}

function arrayBuffer(value: Uint8Array): ArrayBuffer {
  return value.slice().buffer as ArrayBuffer;
}
