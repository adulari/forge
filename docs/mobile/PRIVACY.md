# Forge mobile privacy policy

Effective: 31 July 2026

Forge is an open-source client for a coding-agent daemon you run or explicitly connect to. The app
does not sell personal information, run advertising, or use cross-app tracking.

## Data the app stores on your device

Forge stores the connection records you create, display preferences, app-lock settings, draft
messages, offline queue entries, and limited cached server data needed to make the app work.
Connection bearer tokens are stored through the operating system's secure credential storage.
Content shared into Forge from another app is stored locally as a pending new-session draft until
you submit it. Removing a server deletes its stored token from that device.

## Data sent to your Forge host

When you connect to `forge serve`, the app sends the prompts, answers, attachments, voice
recordings, configuration changes, and control actions you choose to that host. A user-selected
HTTPS tunnel or reverse proxy may transport that traffic and can terminate TLS. Forge Anywhere
instead end-to-end encrypts controller/host content as documented in
[the Forge Anywhere privacy inventory](../anywhere/privacy-data-inventory.md).

## Notifications

Web Push subscriptions go to the connected daemon. Native iOS notifications use Apple Push
Notification service, either through Forge's hosted relay, a relay you configure, or Apple
credentials you provide directly to your own daemon. The public Forge relay receives an opaque
Apple device token and a generic notification or bounded Live Activity status. It does not receive
prompts, transcripts, source files, workspace paths, commands, credentials, or daemon connection
tokens. See [ADR-0012](../architecture/decisions/0012-hosted-apns-relay.md) for the exact boundary.

## Optional anonymous telemetry

Anonymous telemetry is enabled by default and can be disabled in Settings. It records bounded
install, launch, activity, platform, version, distribution, outcome, and coarse performance/usage
counters. It never includes prompts, responses, file contents, paths, repository names, API keys,
account IDs, IP-derived location, or stable device/user identifiers. Full field definitions and
retention are documented in [telemetry.md](../telemetry.md).

## Platform services

Apple, Google, Expo, GitHub, a tunnel provider you select, and a provider used by your Forge host
process data under their own policies when their services are used. Forge requests camera access
only for QR pairing, microphone access only for voice input, photo/document access only when you
attach something, and biometric access only when you enable app lock.

## Deletion and support

Delete a server from Settings to remove its local credential, clear app storage or uninstall Forge
to remove local app data, and use Forge Anywhere's account controls for managed account data.
Host-side sessions and files remain under the control of the host you connected to.

Questions and privacy requests can be filed at
[github.com/adulari/forge/issues](https://github.com/adulari/forge/issues). Do not include tokens,
credentials, private prompts, or source code in a public issue.
