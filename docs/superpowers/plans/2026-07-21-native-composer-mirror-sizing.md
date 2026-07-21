# Native Composer Mirror Sizing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the native iOS multiline composer grow and cap without ever feeding the `TextInput`'s own native content-size events back into its frame.

**Architecture:** Web keeps its existing textarea `scrollHeight` state. Native uses an invisible `Text` mirror in normal Yoga flow to determine the wrapper's intrinsic height; the editable `TextInput` is absolutely overlaid, always scroll-enabled, and has no `onContentSizeChange` handler or JS-driven height. The mirror and input share typography, padding, and width, and a zero-width suffix preserves empty/trailing-newline lines.

**Tech Stack:** React Native 0.86, Expo 57, TypeScript, Vitest, EAS Update.

## Global Constraints

- Native iOS proof must come from the actual native app; web responsive/mobile mode is not evidence.
- Native sizing must not consume `TextInput.onContentSizeChange` and must not set the native wrapper height from a TextInput measurement.
- Native `scrollEnabled` must not toggle as a consequence of measured height.
- Web textarea sizing behavior remains unchanged.
- Preserve native local text state/caret behavior, saved drafts, ghost suggestions, attachments, and one-to-six-line cap.
- Keep #859 open until the production OTA is installed and the focused composer remains stable on native iOS for at least five minutes.
- Do not modify unrelated files.

---

### Task 1: Replace Native Feedback Sizing with a Layout Mirror

**Files:**
- Modify: `mobile/src/components/chat/Composer.tsx`
- Modify: `mobile/src/components/chat/composerSizing.ts`
- Modify: `mobile/src/components/chat/composerSizing.test.ts`
- Create: `docs/superpowers/plans/2026-07-21-native-composer-mirror-sizing.md`

**Interfaces:**
- Consumes: native draft text and the existing composer typography/padding constants.
- Produces: `nativeComposerMirrorText(text: string): string`, a normal-flow native mirror, and an absolutely overlaid native `TextInput`.

- [ ] **Step 1: Write the failing strategy regression**

Replace the obsolete native content-height fixed-point test with tests that require:

```ts
expect(nativeComposerMirrorText("")).toBe("\u200b");
expect(nativeComposerMirrorText("one\ntwo")).toBe("one\ntwo\u200b");
expect(nativeComposerMirrorText("one\n")).toBe("one\n\u200b");
expect(composerUsesNativeMirror("ios")).toBe(true);
expect(composerUsesNativeMirror("android")).toBe(true);
expect(composerUsesNativeMirror("web")).toBe(false);
expect(composerScrollEnabled("ios", COMPOSER_MIN_HEIGHT)).toBe(true);
expect(composerScrollEnabled("ios", COMPOSER_MAX_HEIGHT)).toBe(true);
expect(composerScrollEnabled("web", COMPOSER_MIN_HEIGHT)).toBe(false);
expect(composerScrollEnabled("web", COMPOSER_MAX_HEIGHT)).toBe(true);
```

- [ ] **Step 2: Verify RED**

Run: `npm test -- --run src/components/chat/composerSizing.test.ts`

Expected: FAIL because the mirror/strategy helpers do not exist.

- [ ] **Step 3: Implement the pure strategy helpers**

In `composerSizing.ts`, remove `nativeComposerHeightFromContent` and add:

```ts
export function composerUsesNativeMirror(platform: string): boolean {
  return platform !== "web";
}

export function nativeComposerMirrorText(text: string): string {
  return `${text}\u200b`;
}

export function composerScrollEnabled(platform: string, webHeight: number): boolean {
  return platform === "web" ? webHeight >= COMPOSER_MAX_HEIGHT : true;
}
```

- [ ] **Step 4: Replace native measurement feedback in `Composer`**

Keep the `height` state and `scrollHeight` effect for web only. In the native `inputWrap`:

- remove the explicit `{ height, minHeight }` style;
- set normal-flow `minHeight: MIN_HEIGHT`, `maxHeight: MAX_HEIGHT`, and clipped overflow;
- render a native-only, non-accessible invisible `Text` mirror using `nativeComposerMirrorText(nativeText)`, `type.body`, the same horizontal/vertical padding, and the same available width;
- render the native `TextInput` with `StyleSheet.absoluteFillObject` so it follows the mirror-owned wrapper size;
- remove native `onContentSizeChange` entirely;
- make native scrolling permanently enabled via `composerScrollEnabled`, while retaining the current web threshold;
- align native controls to the bottom without reading a native measured height; keep current web alignment behavior.

The mirror must use `opacity: 0` (not `display: none`), `pointerEvents="none"`, `accessible={false}`, `accessibilityElementsHidden`, and `importantForAccessibility="no-hide-descendants"` so it participates in layout but never duplicates spoken content or input handling.

- [ ] **Step 5: Verify GREEN and complete mobile gate**

Run: `npm test -- --run src/components/chat/composerSizing.test.ts`

Expected: PASS.

Run: `npm run check`

Expected: ESLint, TypeScript, and all mobile tests PASS.

Run: `npx expo export --platform ios --output-dir /tmp/forge-composer-ios-export`

Expected: native iOS JavaScript bundle export PASS. The temporary export is verification output only and must not be committed.

- [ ] **Step 6: Commit**

```bash
git add mobile/src/components/chat/Composer.tsx mobile/src/components/chat/composerSizing.ts mobile/src/components/chat/composerSizing.test.ts docs/superpowers/plans/2026-07-21-native-composer-mirror-sizing.md
git commit -m "fix(mobile): decouple native composer sizing"
```

---

### Task 2: Production Native Verification

**Files:**
- No source changes expected.

**Interfaces:**
- Consumes: merged main commit, production runtime-gated iOS OTA, installed native Forge app.
- Produces: physical native-iOS stability evidence.

- [ ] **Step 1: Merge only after CI and publish a real OTA**

Confirm the EAS publish step itself succeeds and record update group, iOS update ID, runtime, and commit.

- [ ] **Step 2: Verify in native iOS**

On the installed native app, type/wrap from one through six lines, insert and delete explicit newlines including a trailing newline, delete back to one line, move the caret and edit in the middle, then leave the focused composer untouched for at least five minutes.

- [ ] **Step 3: Verify cap and scrolling**

At more than six lines, confirm the composer stays capped, text scrolls, controls remain aligned, the caret stays visible, and no height oscillation occurs.

- [ ] **Step 4: Close only after native evidence passes**

If and only if all native checks pass, record evidence and close #859. Any native failure keeps it open and returns to root-cause investigation.
