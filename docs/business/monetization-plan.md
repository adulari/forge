# Forge Monetization — Execution Plan

*Companion to [monetization-decision.md](monetization-decision.md), which holds the research,
scoring, and citations. This file is the actionable plan. Dates assume a start of 2026-08-01;
shift all gates by the same offset if the start slips.*

## Thesis (one paragraph)

The agent stays free and AGPL forever for individuals. Revenue comes from the **organizational
altitude**: a self-hosted, license-keyed **Forge Control Plane** (central provider keys, per-dev
budget caps, org routing policy, spend + savings dashboards, hash-chained audit export, SSO), plus
**Anywhere Pro** ($6/mo hosted relay + push convenience) as bridge revenue shipped first because
its infrastructure already runs. Everything is gated on adoption: with no free user base, no SKU
matters — Phase 0 is a distribution phase, not a monetization phase.

## Free / paid boundary (commitment)

| Free forever | Paid |
|---|---|
| Agent, TUI, mesh routing + failover, Lattice, skills, workflows, voice | Hosted relay pairing + push (Anywhere Pro) |
| Desktop + mobile apps, self-hosted `forge serve`, self-hosted Anywhere relay | Central key vault, org budget caps, org routing policy |
| `forge blame`, local per-dev spend view, bench | Cross-developer aggregation, savings reports, audit export |
| All of the above at any team size, self-assembled | SSO/SCIM, RBAC, support SLA (Enterprise) |

The boundary is *organizational features*, never capability. Individuals must never hit the paywall.

## SKUs

1. **Anywhere Pro** — $6/mo or $60/yr per user. Hosted APNs push relay, hosted E2E relay with blob
   offload, zero-config tunnel, multi-device, priority OTA channel. Free path (self-hosted relay)
   stays documented and first-class per `docs/anywhere/agpl-service-boundary-checklist.md`.
2. **Control Plane Team** — $15/active-dev/mo, $99/mo floor. Founding design partners: $99/mo flat
   for 12 months.
3. **Control Plane Enterprise** — from $1,000/mo, annual invoice. SSO/SCIM, audit export, SLA.
   Built only when a signed deal demands it.
4. **Commercial license (passive)** — non-AGPL license of the harness/mesh crates, $10K–$50K/yr.
   No build effort; a LICENSING.md and a contact address only.

## Phase 0 — Distribution gate (2026-08-01 → 2026-08-31)

The only phase that matters until it passes.

- [ ] Real launch push: Show HN, r/LocalLLaMA, r/ChatGPTCoding, lobste.rs, X. Angle: the mesh
      cost story ("same model, ~21% cheaper per fix, benchmarked") with reproducible receipts.
- [ ] Opt-in, privacy-respecting active-install signal (no telemetry by default; explicit opt-in).
- [ ] "Forge for Teams" page with visible pricing ($15/dev) and a 2-field design-partner form.
- [ ] 15 interviews with eng leads / platform engineers about agent-spend visibility.

**Pass:** ≥1,000 stars or ≥500 weekly actives, and ≥10 team-page signups.
**Fail:** <150 stars and <50 weekly actives after a genuine launch push → stop; diagnose
positioning; do not start any paid work.

## Phase 1 — Concierge + pre-sell (2026-09-01 → 2026-09-30)

- [ ] Stripe billing + license-key check (shared infrastructure for both SKUs).
- [ ] Anywhere Pro: gate hosted-relay pairing behind entitlement; measure trial-start rate.
- [ ] Recruit 3–5 Control Plane design partners from the Phase 0 form. Concierge delivery:
      manual weekly spend/savings/provenance report generated from their Forge Store DBs +
      hand-maintained org policy file. **Charge $99/mo from day one** — unpaid pilots are not
      validation.

**Pass:** ≥3 paying design partners; Anywhere Pro trial-start ≥2% of weekly actives.
**Kill:** zero orgs pay $99/mo even concierge-grade → org thesis is wrong; fall back to
commercial licensing / services or park monetization and keep building adoption.

## Phase 2 — Build what is paid for (2026-10-01 → 2026-10-31)

- [ ] Automate the concierge work in order of partner demand (report generator first — the
      `routing_decision` table and Store already hold the data).
- [ ] Served org policy file via the existing layered `forge-config` merge.
- [ ] Central key vault (devs never hold raw provider keys).

**Pass:** 3+ partners renew into month 2; pipeline of ≥10 further teams.
**Hard kill:** by 2026-10-31, MRR < $500 *and* weekly actives < 500 → stop all monetization work
for two quarters; Forge is a distribution project until further notice.

## Minimum monetizable product (build list)

Must-have only:

1. License-key check + Stripe billing.
2. Anywhere Pro entitlement gate on hosted-relay pairing.
3. Store-ingest script → weekly org spend/savings/provenance report (concierge tool).
4. Served org policy file.
5. Pricing page + LICENSING.md.

Explicitly deferred (attractive distractions): SSO/SCIM before a signed enterprise deal, hosted
dashboard UI before the concierge report proves the content, marketplace, mesh-as-API,
per-outcome billing, any new agent capability justified "for monetization".

## Standing risks to track

- **Subscription bridging is ToS-hostile for Claude** (enforced 2026-04-04). Never build paid
  functionality on it; keep bridges best-effort and clearly labeled.
- **Vendor absorption**: if Anthropic/OpenAI/GitHub ship org-level *cross-tool* spend governance,
  the open cell closes — retreat to self-host/EU/audit niche or commercial licensing.
- **Solo support capacity** caps Enterprise logos; price higher, take fewer.
- **AGPL service boundary** for hosted components: keep `docs/anywhere/` checklists current so the
  paid hosted tier never poisons the open-source posture.

## Renewal logic

The product must re-justify itself monthly without a sales touch: the spend/savings report
("routing saved you $X vs. list price this month") plus audit exports that procurement has already
filed. Removing Forge recreates the visibility gap — that is the retention mechanism.
