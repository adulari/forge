# Forge Anywhere AGPL/private-service boundary checklist

This is an engineering release gate, not legal advice. Forge is AGPL-3.0-only. Its Anywhere
protocol, envelope cryptography, fixtures, CLI connector, sync client, capsule implementation, and
mobile/web client remain public in the
[`adulari/forge`](https://github.com/adulari/forge) repository. The separately deployed
[`forge-anywhere-service`](https://github.com/adulari/forge-anywhere-service) is private and
independently implements the published network protocol.

## Public side

- [ ] Wire/API behavior needed for an interoperable client/service is normative in
  [`protocol/anywhere-v1.md`](../../protocol/anywhere-v1.md), not hidden in service code.
- [ ] Golden fixture bytes and generation/verification rules are public and language-neutral.
- [ ] Client cryptography, recovery, pairing, revocation, connector allowlist, sync rules, capsule
  safety, and transport code remain AGPL in this repository.
- [ ] A user can still run Forge, local/LAN remote control, direct pairing, and their own
  `forge serve --tunnel` tunnel without the private service or a subscription.
- [ ] Public documentation clearly distinguishes the optional managed service from Forge itself.

## Private service side

- [ ] The service repository has its own Cargo workspace/dependencies and does not import or link
  any `forge-*` crate, workspace path, Git submodule, vendored Forge source, or generated artifact
  containing Forge implementation code.
- [ ] Service envelope parsing/verification is an independent implementation of the published
  protocol.
- [ ] Compatibility tests copy public golden fixture **data**, with attribution/license records;
  they do not copy fixture generator or client implementation code.
- [ ] No source file is copied between repositories. Shared concepts are reimplemented from the
  normative protocol and reviewed for provenance.
- [ ] CI scans dependency manifests, lockfiles, build scripts, submodules, and license inventory for
  accidental Forge linkage before release.
- [ ] Service API errors and JSON schemas remain sufficient for third-party interoperability.
- [ ] Service logs, DB, R2, and backups contain ciphertext/metadata only; privacy is a protocol and
  operational boundary as well as a licensing boundary.

## Deployment separation

- [ ] `forge-anywhere-service` runs as its own user/process on `127.0.0.1:8789`.
- [ ] The existing APNs-only public `forge-relay` stays separate on `127.0.0.1:8788` and `/relay`;
  it is not imported, merged, or broadened into the Anywhere backend.
- [ ] Nginx routes are explicit and do not create an arbitrary proxy into the Forge daemon.
- [ ] Build pipelines, artifact registries, credentials, databases, R2 production/backup buckets,
  and source access controls are independently scoped.
- [ ] Release notes link the public protocol/source and identify the hosted service as optional.

## Review evidence

Record reviewers, date, public/private revisions or release tags, dependency scan result, copied
fixture inventory, and any legal advice. Do not record private source excerpts in the public issue
or PR. The implementation work is tracked in public
[Forge PR #811](https://github.com/adulari/forge/pull/811) and private service
[PR #1](https://github.com/adulari/forge-anywhere-service/pull/1); links may be replaced with stable
release notes after launch.
