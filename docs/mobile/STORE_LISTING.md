# Forge mobile store listing

This file is the version-controlled source draft for App Store Connect, Google Play, review notes,
and screenshots. Store portals remain authoritative for current field limits and required device
sizes; verify those limits at submission time rather than truncating copy silently.

## Shared product facts

- Product name: **Forge**
- Primary category: **Developer Tools**
- Secondary category: **Productivity**
- License: **GNU AGPL v3.0**
- Privacy policy: <https://github.com/adulari/forge/blob/main/docs/mobile/PRIVACY.md>
- Support: <https://github.com/adulari/forge/issues>
- Marketing/source: <https://github.com/adulari/forge>
- Account model: no Forge account is required for direct daemon pairing; Forge Anywhere features
  use their own explicit sign-in.
- Backend model: the app controls a Forge daemon the user runs or explicitly connects to. It does
  not bundle a hosted coding-agent backend.

Do not claim Android native push, end-to-end encryption through a user-selected HTTPS tunnel,
automatic App Store availability, or a publicly verified Android release until those exact gates
have passed.

## Apple App Store draft

Name:

> Forge

Subtitle:

> Your coding agents, anywhere

Promotional text:

> Start, monitor, review, and steer Forge coding sessions from your phone, tablet, browser, or
> desktop—using the providers and daemon you control.

Keywords:

> coding,agent,developer,AI,terminal,git,remote,automation,code review,worktree

Description:

> Forge is the companion app for the open-source Forge coding-agent daemon.
>
> Pair with a daemon you run, then start and follow coding sessions away from the terminal. Review
> live responses and tool activity, answer questions and permission prompts, inspect workspace
> files and diffs, switch branches and worktrees, use terminals, and keep multiple sessions moving
> from one Fleet.
>
> Forge works across providers. Your daemon can route work through its configured model mesh,
> preserve checkpoints and session history, and continue queued Forge Anywhere work when a host
> reconnects.
>
> Native conveniences include QR pairing, secure credential storage, attachments, voice input,
> Share to Forge for text and web links, notifications, and biometric app lock. iOS also supports
> Home Screen widgets and Live Activities.
>
> Forge is self-hosted software, not a bundled cloud coding service. Session content goes to the
> daemon and providers you configure. A tunnel or relay you choose may process limited traffic as
> described in the privacy policy. Anonymous product-interaction telemetry can be disabled in
> Settings.
>
> Forge is free and open source under the GNU AGPL v3.0.

## Google Play draft

Short description:

> Start, monitor, and steer your self-hosted Forge coding agents anywhere.

Long description:

> Forge is the mobile companion for the open-source Forge coding-agent daemon.
>
> Pair with a daemon you run and control coding sessions from Android: create tasks, follow live
> output, answer questions and permission prompts, inspect files and diffs, switch branches and
> worktrees, and keep multiple sessions organized in the Fleet.
>
> Share text or a web link from another app directly into a durable Forge task draft. Use QR
> pairing, secure credential storage, attachments, voice input, biometric app lock, and responsive
> phone, tablet, web, and desktop views.
>
> Forge works with the model providers configured on your daemon, including automatic mesh routing
> and failover. Forge Anywhere can queue encrypted work for a paired host that is temporarily
> offline.
>
> The app does not include a hosted coding backend. Session content goes to the daemon and model
> providers you select. Optional tunnels, notifications, Forge Anywhere, and anonymous telemetry
> have the boundaries documented in the public privacy policy. Telemetry can be disabled in
> Settings.
>
> Forge is free and open source under the GNU AGPL v3.0.

## App Review notes template

Replace every angle-bracket placeholder for the exact submitted build. Never put a durable
production token, valuable repository, personal provider credential, or unrestricted host in
review notes.

> Forge is a client for a user-operated coding-agent daemon; it has no useful offline demo mode.
>
> Review daemon URL: `<SHORT_LIVED_PAIRING_URL>`
>
> The daemon runs in a disposable repository with no valuable credentials or network access. The
> pairing bearer token is valid only for the review window and will be rotated afterward.
>
> Suggested review:
>
> 1. Open Forge and choose “Paste pairing URL.”
> 2. Paste the URL above and connect.
> 3. Open Fleet, tap “Forge a task,” enter `Summarize the demo repository`, and submit.
> 4. Open the resulting session to view streamed output and activity.
> 5. Open History, Workspace, Source control, Diagnostics & updates, and Legal & support.
> 6. From Safari, share a web page to Forge and confirm it appears in a new task draft.
>
> The selected HTTPS tunnel terminates TLS. The default notification relay receives only the
> bounded payload described in the privacy policy; it does not receive prompts, transcripts,
> source files, paths, credentials, or daemon connection tokens.
>
> Contact: `<REVIEW_CONTACT_NAME>`, `<REVIEW_CONTACT_EMAIL>`, `<REVIEW_CONTACT_PHONE>`.

Before opening the review window:

1. Start `forge serve --tunnel` in a disposable repository and environment.
2. Verify the exact URL on a clean physical device.
3. Put that short-lived URL and the contact fields into App Review Information.
4. Rotate the daemon token before and after review with `forge serve --rotate-token`.

## Screenshot matrix

Use production-like seeded data with no real repository names, paths, prompts, tokens, provider
accounts, or notification identifiers. Keep one coherent sample project and session across every
frame. Capture light and dark hero images only when both add information; stores do not need two
near-identical sets.

| Order | Story | Phone | Tablet | Required state |
|---:|---|:---:|:---:|---|
| 1 | Fleet: all coding work at a glance | yes | yes | Several sessions; busy, waiting, completed |
| 2 | Live session: steer and approve | yes | yes | Response, tool activity, question/permission affordance |
| 3 | Workspace: find and inspect code | yes | yes | File tree/search plus readable source |
| 4 | Review: understand a change | yes | yes | Rich diff and one review annotation |
| 5 | Source control: branch safely | yes | optional | Branch/worktree state with no private remote |
| 6 | Remote terminal | yes | yes | Benign terminal command with no private path or output |
| 7 | Providers and routing | yes | optional | Healthy redacted accounts and mesh selection |
| 8 | Anywhere, diagnostics, and privacy | yes | yes | Connected host, compatibility, Legal & support |

Capture targets for the current submission:

- iPhone: the largest portrait class requested by App Store Connect; also capture the smaller
  required class if the portal does not scale it automatically.
- iPad: the largest 13-inch portrait class requested by App Store Connect.
- Android phone: 1080×1920 or higher, 16:9 portrait, without device-frame artwork baked in.
- Android tablet: a native tablet capture if that form factor is enabled in Play Console.

For each target, verify safe-area spacing, keyboard dismissal, readable code/diff text, no clipped
buttons, and no development banners. Re-capture after any visual change affecting the shown screen.

## Submission record

Record these without secrets for each candidate:

| Field | Value |
|---|---|
| Source commit | `<SHA>` |
| Marketing version | `<VERSION>` |
| Apple build number | `<BUILD>` |
| Android version code | `<VERSION_CODE>` |
| Native runtime fingerprint | `<FINGERPRINT>` |
| Privacy policy reviewed | `<DATE / REVIEWER>` |
| Screenshot set reviewed | `<DATE / REVIEWER>` |
| Physical-device smoke test | `<DEVICE / OS / DATE / REVIEWER>` |
