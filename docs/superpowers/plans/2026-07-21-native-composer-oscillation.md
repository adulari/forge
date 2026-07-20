# Native Composer Oscillation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep the native iOS composer at a stable height while preserving correct one-to-six-line growth and the cursor-reset fix from PR #853.

**Architecture:** Treat React Native's native `contentSize.height` as the complete measured height, including the TextInput's vertical inset, and clamp it exactly once before applying it to the wrapper. Keep vertical inset in the native TextInput so UIKit measures it as part of content instead of adding a second synthetic inset after every layout event.

**Tech Stack:** React Native 0.81, Expo 54, TypeScript, Vitest, EAS Update.

## Global Constraints

- The web composer sizing path and `scrollHeight` behavior must remain unchanged.
- Native composer height is 44 points for one line, grows by the measured native content size, and caps at 154 points/six lines.
- The native TextInput must retain the local text state introduced by PR #853; this change must not reintroduce controlled-caret resets.
- No issue may be closed until the production iOS OTA is installed and the composer remains stable during a five-minute native-device observation.

---

### Task 1: Make native content sizing a stable fixed point

**Files:**
- Modify: `mobile/src/components/chat/composerSizing.ts`
- Modify: `mobile/src/components/chat/composerSizing.test.ts`
- Modify: `mobile/src/components/chat/Composer.tsx`

**Interfaces:**
- Consumes: React Native `onContentSizeChange` event field `nativeEvent.contentSize.height`.
- Produces: `nativeComposerHeightFromContent(contentHeight: number): number`.

- [ ] **Step 1: Write the failing fixed-point regression test**

Add this import and test to `composerSizing.test.ts`:

```ts
import {
  clampComposerHeight,
  COMPOSER_MAX_HEIGHT,
  COMPOSER_MAX_LINES,
  COMPOSER_MIN_HEIGHT,
  nativeComposerHeightFromContent,
} from "./composerSizing";

it("does not compound native vertical inset across repeated layout measurements", () => {
  for (const measuredHeight of [44, 66, 88, 110, 132, 154]) {
    expect(nativeComposerHeightFromContent(measuredHeight)).toBe(measuredHeight);
  }
  expect(nativeComposerHeightFromContent(22)).toBe(COMPOSER_MIN_HEIGHT);
});
```

- [ ] **Step 2: Run the regression test and verify RED**

Run:

```bash
cd mobile
npm test -- --run src/components/chat/composerSizing.test.ts
```

Expected: FAIL because `nativeComposerHeightFromContent` is not exported.

- [ ] **Step 3: Implement the stable native measurement mapping**

Add to `composerSizing.ts`:

```ts
export function nativeComposerHeightFromContent(contentHeight: number): number {
  return clampComposerHeight(contentHeight);
}
```

In `Composer.tsx`, import `nativeComposerHeightFromContent`, delete `INPUT_VERTICAL_PADDING`, replace:

```ts
const next = clampComposerHeight(contentHeight + INPUT_VERTICAL_PADDING);
```

with:

```ts
const next = nativeComposerHeightFromContent(contentHeight);
```

Restore the native input inset in `styles.input`:

```ts
paddingVertical: (MIN_HEIGHT - LINE_HEIGHT) / 2,
```

- [ ] **Step 4: Verify GREEN and run the mobile suite**

Run:

```bash
cd mobile
npm test -- --run src/components/chat/composerSizing.test.ts
npm run check
```

Expected: the sizing regression passes and the complete mobile lint, typecheck, and test suite passes.

- [ ] **Step 5: Commit**

```bash
git add mobile/src/components/chat/composerSizing.ts mobile/src/components/chat/composerSizing.test.ts mobile/src/components/chat/Composer.tsx docs/superpowers/plans/2026-07-21-native-composer-oscillation.md
git commit -m "fix(mobile): stabilize native composer height"
```

### Task 2: Production-native verification gate

**Files:**
- No source files.

**Interfaces:**
- Consumes: the production iOS OTA automatically published after the Task 1 PR merges to `main`.
- Produces: live verification evidence attached to the corresponding GitHub issue/PR.

- [ ] **Step 1: Verify the production OTA workflow**

Confirm the `eas update (production / ios)` workflow for the merge commit completes successfully and reports a published update on the `production` channel.

- [ ] **Step 2: Exercise the native iOS composer**

On the installed Forge iOS app, open an active session, type one through six lines, delete back to one line, then leave the focused composer open for five minutes.

Expected: height changes only when the text's measured line count changes, remains stable while idle, caps at six lines with scrolling, and the caret remains at the insertion point.

- [ ] **Step 3: Record the gate result**

Only after Step 2 passes, add the device/OTA verification result to the issue and close it. If it fails, keep the issue open and capture the exact text, line count, and observed height transition pattern.
