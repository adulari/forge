# EAS OTA Neutral Path Classification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish OTA-safe iOS changes even when the same main push contains documentation or other non-mobile files, while still blocking native/mobile-build changes.

**Architecture:** Extract file classification from workflow YAML into a locally testable shell classifier with three states: OTA content present, native/config unsafe, and neutral. The workflow consumes those outputs and exposes blocked/no-content publication as explicit steps instead of claiming a successful publish.

**Tech Stack:** Bash, GitHub Actions YAML, EAS Update.

## Global Constraints

- `mobile/src/**` and `mobile/assets/**` are OTA-safe content.
- Native iOS/Android files and mobile dependency/build/config files remain OTA-unsafe.
- Documentation, Rust source, repository metadata, Desktop Tauri source, and workflow/helper files are OTA-neutral.
- Manual main-only recovery remains allowed and runtime-version-gated.
- Never publish an OTA when an unsafe mobile native/config path is in the push range.
- Keep #861 open until an automatic main push containing OTA-safe mobile content plus at least one neutral path reaches the real EAS publish step.

---

### Task 1: Testable OTA Safety Classifier and Workflow Integration

**Files:**
- Create: `scripts/ci/eas-ota-safety.sh`
- Create: `scripts/ci/test-eas-ota-safety.sh`
- Modify: `.github/workflows/eas-update.yml`
- Modify: `.github/workflows/ci.yml`
- Create: `docs/superpowers/plans/2026-07-21-eas-ota-neutral-paths.md`

**Interfaces:**
- Consumes: path arguments for local tests, or `EVENT_NAME`, `BASE_SHA`, and `HEAD_SHA` in GitHub Actions.
- Produces: `safe=true|false` and `ota_changed=true|false` in `$GITHUB_OUTPUT`.

- [ ] **Step 1: Write the failing shell regression**

Create `scripts/ci/test-eas-ota-safety.sh` following the scratch-output pattern in `scripts/ci/test-changed-groups.sh`. It must execute `scripts/ci/eas-ota-safety.sh` and assert:

```text
mobile/src/app.tsx + docs/plan.md                 => safe=true,  ota_changed=true
mobile/assets/icon.png + crates/core/src/lib.rs  => safe=true,  ota_changed=true
docs/plan.md only                                => safe=true,  ota_changed=false
mobile/src/app.tsx + mobile/ios/Info.plist       => safe=false, ota_changed=true
mobile/src/app.tsx + mobile/package-lock.json    => safe=false, ota_changed=true
workflow_dispatch                                => safe=true,  ota_changed=true
```

- [ ] **Step 2: Verify RED**

Run: `bash scripts/ci/test-eas-ota-safety.sh`

Expected: FAIL because `scripts/ci/eas-ota-safety.sh` does not exist.

- [ ] **Step 3: Implement the classifier**

Create an executable Bash classifier that initializes `safe=true`, `ota_changed=false`, and an `unsafe` array. Classify paths with this ordering:

```bash
case "$path" in
  mobile/src/*|mobile/assets/*)
    ota_changed=true
    ;;
  mobile/ios/*|mobile/android/*|mobile/plugins/*|mobile/app.json|mobile/app.config.*|mobile/eas.json|mobile/package.json|mobile/package-lock.json|mobile/PrivacyInfo.xcprivacy|mobile/metro.config.js)
    unsafe+=("$path")
    ;;
  *)
    # Neutral: does not enter the iOS OTA bundle or native runtime.
    ;;
esac
```

When `EVENT_NAME == workflow_dispatch`, set both booleans true without diffing. Otherwise classify explicit arguments, or a NUL-delimited `git diff --name-only -z "$BASE_SHA" "$HEAD_SHA"`. If `unsafe` is non-empty, set `safe=false`. Append both outputs to `${GITHUB_OUTPUT:-/dev/stdout}` and log every unsafe path.

- [ ] **Step 4: Verify classifier GREEN**

Run: `chmod +x scripts/ci/eas-ota-safety.sh scripts/ci/test-eas-ota-safety.sh && bash scripts/ci/test-eas-ota-safety.sh`

Expected: `EAS OTA safety classification passed`.

- [ ] **Step 5: Integrate the classifier into the workflow**

Replace the inline range classifier in `.github/workflows/eas-update.yml` with:

```yaml
- name: Guard OTA-safe diff
  id: guard
  working-directory: .
  env:
    EVENT_NAME: ${{ github.event_name }}
    BASE_SHA: ${{ github.event.before }}
    HEAD_SHA: ${{ github.sha }}
  run: bash scripts/ci/eas-ota-safety.sh
```

Gate setup, install, runtime validation, and publish on both `steps.guard.outputs.safe == 'true'` and `steps.guard.outputs.ota_changed == 'true'`. Keep an explicit `Skip incompatible OTA` step for `safe != 'true'`, and add `No OTA content to publish` for `safe == 'true' && ota_changed != 'true'`; both must write a clear summary to `$GITHUB_STEP_SUMMARY`.

- [ ] **Step 6: Run the regression in CI’s lightweight detector job**

Add `bash scripts/ci/test-eas-ota-safety.sh` after the changed-file classifier step in `.github/workflows/ci.yml`. This deterministic sub-second test does not require Node, Rust, EAS credentials, or a heavy runner.

- [ ] **Step 7: Verify workflow syntax and focused gates**

Run: `bash scripts/ci/test-eas-ota-safety.sh && bash scripts/ci/test-changed-groups.sh`

Expected: both classifiers pass.

Run: `actionlint .github/workflows/eas-update.yml .github/workflows/ci.yml`

Expected: PASS. If `actionlint` is unavailable locally, record that explicitly and use the repository’s CI workflow parser as the required gate before merge.

- [ ] **Step 8: Commit**

```bash
git add scripts/ci/eas-ota-safety.sh scripts/ci/test-eas-ota-safety.sh .github/workflows/eas-update.yml .github/workflows/ci.yml docs/superpowers/plans/2026-07-21-eas-ota-neutral-paths.md
git commit -m "fix(ci): ignore neutral files in OTA safety guard"
```
