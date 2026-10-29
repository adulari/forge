# Versioning

Forge ships several artifacts from one repository. They do not share one version number, and the
rules for "is this client compatible with that daemon" live in different places. This page is the
map.

## The numbers

| Number | Where it lives | Who changes it |
| --- | --- | --- |
| **Release version** (`2.24.1`) | `[workspace.package] version` in `Cargo.toml`, `Cargo.lock`, `mobile/src-tauri/tauri.conf.json`, the `## [X.Y.Z]` heading in both changelogs | Release CI stamps it from the tag (`scripts/ci/stamp-release-version.sh`); nobody edits it by hand |
| **Remote protocol** (`PROTOCOL_VERSION`, currently 10) | `mobile/src/lib/remoteProtocol.ts`, mirrored by the daemon's snapshot contract | Bumped by a PR that changes what the daemon and clients must agree on |
| **Mobile client version** (`1.0.1`) | `version` in `mobile/app.config.ts` | Hand-edited for a store build; the phone apps do not follow the release version |
| **Native runtime version** | Expo: `runtimeVersion: { policy: "fingerprint" }`; desktop: the 16-hex hash from `mobile/src-tauri/runtime_version.rs` | Computed, never typed |
| **Android minSdk** | The React Native default, see below | Moves with the React Native upgrade |

The release version and the protocol version are independent on purpose. A daemon and a client from
different releases talk to each other as long as they agree on the protocol; the release version
only decides which features the user sees and which notes `GET /api/changelog` returns.

## Release version

A release is a tag plus one dispatch (`docs/ci/releasing.md`). The version is derived from the tag,
so a branch never carries a half-bumped number. The daemon embeds `CHANGELOG.md` at build time and
`forge --version` reads `CARGO_PKG_VERSION`, so both always describe the binary that is running.

The desktop app shares this number with the daemon (`tauri.conf.json`). On desktop the app reports
the installed bundle version; on phones and the web it reports the client version above.
`mobile/src/lib/appVersionCore.ts` is the one place that chooses between them.

### Release notes

`## [Unreleased]` collects entries under the usual `### Added` / `### Changed` / `### Fixed`
headings. Root `CHANGELOG.md` and `crates/forge-cli/CHANGELOG.md` must stay byte-identical (the
second is the copy the crate compiles in).

A release section may start with two optional comment lines, invisible on GitHub, that the What's
New panel and the post-update sheet render as a hero block:

```markdown
## [2.25.0] - 2026-11-02
<!-- headline: Threads that remember what they were doing -->
<!-- chips: Gallery | Command palette | Goal docks -->
```

`headline` is one line. `chips` is a `|`-separated list of at most four short labels. Release CI
rolls the whole `[Unreleased]` section, so write both lines directly under `## [Unreleased]`
before cutting the release.

## Remote protocol

The protocol number gates the live session socket and the REST surface a client may call. A client
that sees a frame it does not recognise drops it rather than guessing (`isValidSnapshotFrame`).
New fields are added as optional and additive; a field's absence means "an older daemon". Removing
or reinterpreting a field requires a protocol bump and a release note.

## Over-the-air updates and native compatibility

JavaScript can change without a store release; native code cannot.

- **Phones** use EAS Update (`mobile/EAS_UPDATE.md`). A bundle is served only to a binary whose
  fingerprint runtime version matches, so any native change (a module, an entitlement, a config
  plugin, `ios.*` or `android.*` config) forces a new binary build.
- **Desktop** uses signed UI bundles (`docs/features/desktop-ota.md`) gated on the runtime hash of
  the Rust shell. A change under `mobile/src-tauri/` needs a full release.
- **CI guard.** The `mobile-typecheck` workflow runs `expo prebuild --no-install` so a config or
  plugin change that breaks native project generation fails the pull request instead of the next
  store build.

## Android minSdk policy

The project does not set `minSdkVersion` itself. It takes the value React Native ships for the
installed version (`node_modules/react-native/gradle/libs.versions.toml`: minSdk 24, i.e.
Android 7.0, with compile and target SDK 36 at React Native 0.86).

- Raising the floor is a decision to take with a React Native or Expo SDK upgrade, not in isolation.
  Record it in the changelog under `### Changed` because it drops devices from the Play Store
  listing.
- Do not pin a value below the React Native default; the native modules in the tree are built
  against it.
- If a dependency needs a higher floor, prefer a different dependency over raising it for one
  feature.
