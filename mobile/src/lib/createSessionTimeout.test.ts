/// <reference types="vite/client" />

// api.ts pulls in the RN fetch surface, so this repo's convention for asserting on it is the raw
// source (see components/chat/Composer.nativeLayout.test.ts).
import { describe, expect, it } from "vitest";

import apiSource from "./api.ts?raw";

const source = apiSource.replace(/\s+/g, " ");

describe("createSession request deadline", () => {
  it("gives session creation its own, longer timeout", () => {
    // Spawning a session builds a workspace, starts the session and runs its first turn: 4.4s
    // measured over loopback, more over the encrypted relay, against a 15s default that aborted
    // a create the daemon was in fact carrying out.
    expect(source).toMatch(/const CREATE_SESSION_TIMEOUT_MS = 90_000;/);
    expect(source).toMatch(/"\/api\/sessions", \{ method: "POST", body: JSON\.stringify\(body\) \}, CREATE_SESSION_TIMEOUT_MS,/);
  });
});
