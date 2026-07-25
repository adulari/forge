// Request-body serialisation for the Anywhere bridge.
//
// A relayed request travels as literal bytes (`AnywhereBridgeRequest.body`), so the body has to be
// encoded on the device before it can be sealed into an envelope. The obvious route — hand `init`
// to `new Request(...)` and read `.arrayBuffer()` — works on web and in Node, where `Request` is
// spec-compliant, but silently fails on React Native for the one body shape that matters most:
// RN's `Request` comes from whatwg-fetch, whose `arrayBuffer()` delegates to `blob()`, and that
// throws `could not read FormData body as blob`. whatwg-fetch never implements a multipart encoder
// — on RN only the *platform* fetch can serialise FormData, and the relay is not the platform
// fetch. That is why voice upload and every file/image attachment failed before leaving an
// Anywhere-paired phone.
//
// Expo's WinterCG fetch carries an encoder of its own (expo/src/winter/fetch/convertFormData.ts),
// but it is private to that module and absent from the web/Tauri bundles, so the encoding lives
// here instead: one implementation every target shares, and one that is directly testable.
//
// Part values differ per platform and all of these must work:
//   - `string`                                      — scalar fields
//   - `Blob`/`File`                                 — web (attach.ts formDataFromWebFiles, paste)
//   - `{ bytes(): Promise<Uint8Array>, name, type }` — native (voice.ts, attach.ts), the shape
//     expo's own encoder accepts, because RN's `{uri,name,type}` shorthand only means anything
//     inside RN's networking layer and never reaches us as readable bytes.

const CRLF = "\r\n";
const encoder = new TextEncoder();

export interface EncodedRequestBody {
  bytes: Uint8Array;
  /** Set only when the body dictates its own framing (a multipart boundary, a form encoding). */
  contentType?: string;
}

export interface EncodedMultipart {
  bytes: Uint8Array;
  boundary: string;
}

/** RN's own `FormData.getParts()` part shape: the field name is folded in as `fieldName`. */
interface ReactNativeFormDataPart {
  fieldName: string;
  string?: string;
}

export async function encodeRequestBody(body: unknown): Promise<EncodedRequestBody> {
  if (body == null) return { bytes: new Uint8Array() };
  if (typeof body === "string") return { bytes: encoder.encode(body) };
  if (body instanceof ArrayBuffer) return { bytes: new Uint8Array(body) };
  if (ArrayBuffer.isView(body)) {
    return { bytes: new Uint8Array(body.buffer, body.byteOffset, body.byteLength) };
  }
  if (typeof URLSearchParams !== "undefined" && body instanceof URLSearchParams) {
    return {
      bytes: encoder.encode(body.toString()),
      contentType: "application/x-www-form-urlencoded;charset=UTF-8",
    };
  }
  if (typeof FormData !== "undefined" && body instanceof FormData) {
    const { bytes, boundary } = await encodeMultipart(body);
    return { bytes, contentType: `multipart/form-data; boundary=${boundary}` };
  }
  if (typeof body === "object") {
    // A lone Blob/File body. `type` is the platform's own default content-type for it.
    const bytes = await readBytes(body);
    if (bytes) return { bytes, contentType: stringProperty(body, "type") || undefined };
  }
  throw new Error("Forge Anywhere cannot serialise this request body");
}

/**
 * Serialise a FormData into a multipart/form-data payload.
 *
 * Header escaping follows the WHATWG rule browsers implement — only CR, LF and `"` are
 * percent-escaped, everything else rides as raw UTF-8. That matters: the daemon reads the audio
 * format hint from the part's filename first (serve.rs voice_transcribe), so a filename must
 * survive byte-for-byte rather than being URI-encoded.
 */
export async function encodeMultipart(
  form: FormData,
  boundary: string = createBoundary(),
): Promise<EncodedMultipart> {
  const chunks: Uint8Array[] = [];
  for (const [field, value] of formDataEntries(form)) {
    const disposition = `Content-Disposition: form-data; name="${escapeHeaderValue(field)}"`;
    if (typeof value === "string") {
      chunks.push(encoder.encode(`--${boundary}${CRLF}${disposition}${CRLF}${CRLF}${value}${CRLF}`));
      continue;
    }
    // Browsers name an unnamed Blob part "blob" and fall back to application/octet-stream; match
    // them, so an Anywhere upload is indistinguishable from a direct one on the wire.
    const filename = stringProperty(value, "name") || "blob";
    const type = stringProperty(value, "type") || "application/octet-stream";
    chunks.push(
      encoder.encode(
        `--${boundary}${CRLF}${disposition}; filename="${escapeHeaderValue(filename)}"${CRLF}`
          + `Content-Type: ${type}${CRLF}${CRLF}`,
      ),
    );
    chunks.push(await partBytes(value, field));
    chunks.push(encoder.encode(CRLF));
  }
  chunks.push(encoder.encode(`--${boundary}--${CRLF}`));
  return { bytes: concat(chunks), boundary };
}

const BOUNDARY_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/** A boundary only has to be absent from the payload, so `Math.random` is an acceptable source
 * where the platform has no WebCrypto (Hermes has none unless a polyfill is installed). */
export function createBoundary(): string {
  const raw = new Uint8Array(24);
  const webCrypto = (globalThis as { crypto?: Crypto }).crypto;
  if (typeof webCrypto?.getRandomValues === "function") webCrypto.getRandomValues(raw);
  else for (let index = 0; index < raw.length; index += 1) raw[index] = Math.floor(Math.random() * 256);
  let boundary = "----ForgeFormBoundary";
  for (const byte of raw) boundary += BOUNDARY_ALPHABET[byte % BOUNDARY_ALPHABET.length];
  return boundary;
}

function formDataEntries(form: FormData): [string, unknown][] {
  if (typeof form.entries === "function") {
    return Array.from(form.entries() as Iterable<[string, unknown]>);
  }
  // A React Native FormData that expo's WinterCG patch has not touched has no iteration API at
  // all — `getParts()` is its only reader.
  const legacy = form as unknown as { getParts?: () => ReactNativeFormDataPart[] };
  if (typeof legacy.getParts === "function") {
    return legacy.getParts().map((part) => [part.fieldName, part.string ?? part]);
  }
  throw new Error("Forge Anywhere cannot read this FormData implementation");
}

async function partBytes(value: unknown, field: string): Promise<Uint8Array> {
  const bytes = await readBytes(value);
  if (bytes) return bytes;
  if (stringProperty(value, "uri")) {
    throw new Error(
      `FormData field "${field}" uses React Native's {uri} shorthand, which only the platform `
        + "fetch can read — append a value exposing bytes() instead (see voice.ts / attach.ts)",
    );
  }
  throw new Error(`FormData field "${field}" holds a value Forge Anywhere cannot serialise`);
}

/** `null` when `value` is not a readable binary container. */
async function readBytes(value: unknown): Promise<Uint8Array | null> {
  const source = value as {
    bytes?: () => Promise<Uint8Array> | Uint8Array;
    arrayBuffer?: () => Promise<ArrayBuffer>;
  } | null;
  // `bytes()` first: it is what expo's File and the native recorder/picker adapters expose, and
  // on RN a real Blob has neither method (its bytes only exist in the native blob registry).
  if (typeof source?.bytes === "function") {
    const result = await source.bytes();
    return result instanceof Uint8Array ? result : new Uint8Array(result);
  }
  if (typeof source?.arrayBuffer === "function") return new Uint8Array(await source.arrayBuffer());
  return null;
}

function stringProperty(value: unknown, key: string): string {
  const property = (value as Record<string, unknown> | null)?.[key];
  return typeof property === "string" ? property : "";
}

function escapeHeaderValue(value: string): string {
  return value.replace(/\r/g, "%0D").replace(/\n/g, "%0A").replace(/"/g, "%22");
}

function concat(chunks: Uint8Array[]): Uint8Array {
  const total = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
  const out = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.length;
  }
  return out;
}
