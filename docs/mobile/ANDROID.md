# Native Android distribution

Forge's shared Expo app can be distributed as an installable APK or a Google Play AAB through
`.github/workflows/mobile-android.yml`.

## One-time repository setup

1. Keep the existing `EXPO_TOKEN` Actions secret configured.
2. Create a Google Play service account with release access to `dev.adulari.forge`.
3. Store the complete service-account JSON as the Actions secret
   `GOOGLE_SERVICE_ACCOUNT_KEY_JSON`.

The Google credential is written only to the workflow runner, removed in the final step, and
ignored by git locally.

## Internal APK

Run **Actions → mobile-android → Run workflow**, choose `preview`, and leave Play submission off.
The completed workflow exposes `forge-android-preview` as an installable APK artifact.

## Google Play internal testing

Run the workflow with `production` and enable `submit_to_play`. EAS builds a signed AAB with an
auto-incremented version code, then submits that exact build to the Play internal track. A missing
credential or a non-production profile fails loudly before submission.

Tags matching `android-v*` build the production AAB and attach it to the matching GitHub Release;
tag builds do not automatically submit to Play.
