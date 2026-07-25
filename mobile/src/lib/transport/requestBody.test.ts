import { describe, expect, it } from "vitest";

import { createBoundary, encodeMultipart, encodeRequestBody } from "./requestBody";

const BOUNDARY = "----ForgeTestBoundary";

function text(bytes: Uint8Array): string {
  return new TextDecoder().decode(bytes);
}

/** A FormData that only exposes `entries()`, like expo's patched React Native one. */
function rnFormData(entries: [string, unknown][]): FormData {
  return { entries: () => entries[Symbol.iterator]() } as unknown as FormData;
}

/** A bare React Native FormData: no iteration API, only `getParts()`. */
function bareRnFormData(parts: object[]): FormData {
  return { getParts: () => parts } as unknown as FormData;
}

interface ParsedPart {
  name: string;
  type: string;
  text(): Promise<string>;
}

/** This project's lib set types `Response.formData()` against a FormData declaration with no
 * reader methods, so the parsed form is read through a structural view. */
async function parseMultipart(
  bytes: Uint8Array,
  contentType: string,
): Promise<{ get(name: string): ParsedPart | string | null }> {
  const form = await new Response(bytes as unknown as BodyInit, {
    headers: { "content-type": contentType },
  }).formData();
  return form as unknown as { get(name: string): ParsedPart | string | null };
}

/** The value voice.ts and attach.ts append on native. */
function bytesPart(name: string, type: string, body: string) {
  return { bytes: async () => new TextEncoder().encode(body), name, type };
}

describe("encodeMultipart", () => {
  it("serialises a React Native bytes-part that no RN Request can encode", async () => {
    const form = rnFormData([["file", bytesPart("voice.wav", "audio/wav", "RIFFdata")]]);
    const { bytes, boundary } = await encodeMultipart(form, BOUNDARY);
    expect(boundary).toBe(BOUNDARY);
    expect(text(bytes)).toBe(
      `--${BOUNDARY}\r\n`
        + 'Content-Disposition: form-data; name="file"; filename="voice.wav"\r\n'
        + "Content-Type: audio/wav\r\n\r\n"
        + "RIFFdata\r\n"
        + `--${BOUNDARY}--\r\n`,
    );
  });

  it("serialises web Blob and File parts with their filename and type", async () => {
    const form = new FormData();
    form.append("file", new Blob([new Uint8Array([1, 2, 3])], { type: "audio/wav" }), "voice.wav");
    form.append("files", new File(["png-bytes"], "shot.png", { type: "image/png" }));
    const { bytes } = await encodeMultipart(form, BOUNDARY);
    const body = text(bytes);
    expect(body).toContain('name="file"; filename="voice.wav"\r\nContent-Type: audio/wav');
    expect(body).toContain('name="files"; filename="shot.png"\r\nContent-Type: image/png');
    expect(body).toContain("png-bytes");
  });

  it("keeps field order and encodes string fields as UTF-8", async () => {
    const form = rnFormData([
      ["language", "nl"],
      ["file", bytesPart("voice.wav", "audio/wav", "a")],
      ["note", "héllo"],
    ]);
    const { bytes } = await encodeMultipart(form, BOUNDARY);
    const body = text(bytes);
    expect(body.indexOf('name="language"')).toBeLessThan(body.indexOf('name="file"'));
    expect(body.indexOf('name="file"')).toBeLessThan(body.indexOf('name="note"'));
    expect(body).toContain('Content-Disposition: form-data; name="language"\r\n\r\nnl\r\n');
    expect(body).toContain("héllo");
    // A scalar field carries no Content-Type, exactly as a browser emits it.
    expect(body).not.toContain('name="language"\r\nContent-Type');
  });

  it("escapes quotes and newlines instead of letting them forge a header", async () => {
    const form = rnFormData([['fi"eld', bytesPart('a"b\r\nc.wav', "audio/wav", "x")]]);
    const { bytes } = await encodeMultipart(form, BOUNDARY);
    expect(text(bytes)).toContain(
      'Content-Disposition: form-data; name="fi%22eld"; filename="a%22b%0D%0Ac.wav"',
    );
  });

  it("defaults an unnamed, untyped part the way browsers do", async () => {
    const form = rnFormData([["file", { bytes: async () => new Uint8Array([7]) }]]);
    const { bytes } = await encodeMultipart(form, BOUNDARY);
    expect(text(bytes)).toContain(
      'Content-Disposition: form-data; name="file"; filename="blob"\r\n'
        + "Content-Type: application/octet-stream",
    );
  });

  it("reads a bare React Native FormData through getParts()", async () => {
    const form = bareRnFormData([
      { fieldName: "language", string: "en" },
      { fieldName: "file", ...bytesPart("voice.wav", "audio/wav", "abc") },
    ]);
    const { bytes } = await encodeMultipart(form, BOUNDARY);
    const body = text(bytes);
    expect(body).toContain('name="language"\r\n\r\nen\r\n');
    expect(body).toContain('name="file"; filename="voice.wav"');
    expect(body).toContain("abc");
  });

  it("rejects React Native's {uri} shorthand with an actionable message", async () => {
    const form = rnFormData([["file", { uri: "file:///tmp/voice.wav", name: "voice.wav", type: "audio/wav" }]]);
    await expect(encodeMultipart(form, BOUNDARY)).rejects.toThrow("{uri} shorthand");
  });

  it("rejects a FormData implementation it cannot read", async () => {
    await expect(encodeMultipart({} as FormData, BOUNDARY)).rejects.toThrow("cannot read this FormData");
  });

  it("produces a body a spec-compliant multipart parser reads back", async () => {
    const form = rnFormData([
      ["language", "en"],
      ["file", bytesPart("voice.wav", "audio/wav", "RIFF....WAVE")],
    ]);
    const { bytes, boundary } = await encodeMultipart(form);
    const parsed = await parseMultipart(bytes, `multipart/form-data; boundary=${boundary}`);
    expect(parsed.get("language")).toBe("en");
    const file = parsed.get("file") as ParsedPart;
    expect(file.name).toBe("voice.wav");
    expect(file.type).toBe("audio/wav");
    expect(await file.text()).toBe("RIFF....WAVE");
  });
});

describe("createBoundary", () => {
  it("produces a distinct token-safe boundary each call", () => {
    const first = createBoundary();
    const second = createBoundary();
    expect(first).not.toBe(second);
    expect(first).toMatch(/^----ForgeFormBoundary[A-Za-z0-9]{24}$/);
  });
});

describe("encodeRequestBody", () => {
  it("returns an empty body for a bodyless request", async () => {
    await expect(encodeRequestBody(undefined)).resolves.toEqual({ bytes: new Uint8Array() });
    await expect(encodeRequestBody(null)).resolves.toEqual({ bytes: new Uint8Array() });
  });

  it("passes strings, ArrayBuffers and views through without inventing a content-type", async () => {
    const fromString = await encodeRequestBody('{"a":1}');
    expect(text(fromString.bytes)).toBe('{"a":1}');
    expect(fromString.contentType).toBeUndefined();

    const source = new Uint8Array([1, 2, 3, 4]);
    expect((await encodeRequestBody(source.buffer)).bytes).toEqual(source);
    expect((await encodeRequestBody(source.subarray(1, 3))).bytes).toEqual(new Uint8Array([2, 3]));
  });

  it("labels a FormData body with the boundary it actually used", async () => {
    const form = new FormData();
    form.append("file", new Blob(["wav"], { type: "audio/wav" }), "voice.wav");
    const { bytes, contentType } = await encodeRequestBody(form);
    const boundary = /boundary=(.+)$/.exec(contentType ?? "")?.[1];
    expect(boundary).toBeTruthy();
    expect(text(bytes).startsWith(`--${boundary}\r\n`)).toBe(true);
    expect(text(bytes).endsWith(`--${boundary}--\r\n`)).toBe(true);
  });

  it("encodes URLSearchParams and a lone Blob with their own content-type", async () => {
    const params = await encodeRequestBody(new URLSearchParams({ a: "1", b: "two" }));
    expect(text(params.bytes)).toBe("a=1&b=two");
    expect(params.contentType).toBe("application/x-www-form-urlencoded;charset=UTF-8");

    const blob = await encodeRequestBody(new Blob(["hi"], { type: "text/plain" }));
    expect(text(blob.bytes)).toBe("hi");
    expect(blob.contentType).toBe("text/plain");
  });

  it("rejects a body it cannot serialise", async () => {
    await expect(encodeRequestBody({ nope: true })).rejects.toThrow("cannot serialise");
  });
});
