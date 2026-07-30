# Forge Monetization — Decision Document

*Prepared 2026-07-30. Web research performed 2026-07-30; repository facts verified against `adulari/forge` at commit `e29cb811`. Claims marked **[est.]** are third-party estimates, not confirmed figures. Claims marked **[repo]** were verified directly in the repository.*

---

## 1. Executive verdict

**Primary thesis: sell cost-governed, auditable agent execution to engineering organizations — a paid "Forge Control Plane" layered on a permanently free agent — and do not attempt it until Forge has a real free user base.**

- **Customer:** the engineering manager / platform lead at a 10–200-developer company whose AI coding spend just became variable and unaccountable (Copilot moved to usage billing June 2026; Cursor, Claude Code, and every agent now bill by consumption).
- **Paid outcome:** "I can see, cap, route, and audit what every developer's agents spend and do — and I provably spend less than list price."
- **Exact offer:** a self-hosted, license-keyed org server ("Forge Control Plane"): central provider keys, per-developer/per-team budget caps, org-wide mesh routing policy (including `credit_mode = strict`), aggregated spend dashboards with per-task provenance (`forge blame` at org level), hash-chained audit export for SIEM/SOC2, SSO/RBAC. The agent, mesh, Lattice, Anywhere client — everything a single developer touches — stays free and AGPL.
- **Pricing model:** per-active-developer/month with an org floor. Founding price ~$15/active dev/mo, $99/mo floor; Enterprise tier (SSO/SCIM, audit export, SLA) $1,000–$2,500/mo. Anchored to LiteLLM ($250/mo → ~$30K/yr), Copilot Business ($19/seat), CodeRabbit ($24/user).
- **Why this is the best option:** it monetizes Forge's only genuinely differentiated, already-built asset — benchmark-ranked cost-tier routing plus a complete per-turn audit trail (routing decisions, costs, provenance are already persisted in the Store **[repo]**) — at the altitude where budget actually exists. Individual developers pay for inference, not harness software (Cline: ~5M installs, only ~$5M ARR **[est.]**). Organizations pay for control: FinOps teams doing AI-spend management jumped from 31% to 98% in two years, and governance platforms command $4K–$15K/mo.

**Secondary, immediate:** ship **Forge Anywhere Pro** ($6–8/mo) as bridge revenue — it is the only offer whose hosted infrastructure already exists **[repo: forge-relay]** — but treat it as a funnel and a willingness-to-pay probe, not the business. Its realistic ceiling is low thousands of MRR.

**Gate that overrides everything:** Forge has **8 GitHub stars and 0 forks** six weeks after publication **[repo]**. There is no monetization strategy that survives zero users. The first 90 days are a distribution problem wearing a monetization costume. The validation plan (§8) reflects this.

---

## 2. Uncomfortable truth

Four things the founder is likely overvaluing, stated bluntly:

1. **Subscription bridging — the README's headline "no other agent does this" feature — is now a legal liability, not a moat.** Anthropic explicitly banned subscription OAuth in third-party tools (first blocked January 2026, terms updated February 20, 2026, fully enforced April 4, 2026). Forge's bridge drives the official CLI rather than raw OAuth tokens, which is at best a grey area the vendor has shown active willingness to close. You cannot build a paid product on a capability the upstream vendor is actively hunting. Cline already got burned on exactly this. Any monetization thesis resting on "bring your Claude Max plan" has a kill switch owned by Anthropic.

2. **Technical superiority does not convert.** "50% more bugs fixed than the raw CLI" and 324 conformance tests are engineering achievements, not purchase drivers. Aider is excellent, open source, widely admired — and monetized nothing before drifting into maintenance mode. Cline converted 5M installs into ~$5M ARR **[est.]** — a ratio of roughly $1 per install per year. Enthusiasm, stars, and installs are not willingness to pay.

3. **The individual developer is the wrong buyer for harness software.** Every price point individuals accept ($10 OpenCode Go, $20 Copilot Pro/Omnara, $20–200 Cursor/Claude) buys *inference* — the scarce good — or a hosted convenience. Nobody charges individuals for the open-source harness itself, because OpenCode (171K stars), Cline, and Aider set that price at zero forever. Forge cannot out-free them, and at 8 stars it cannot out-distribute them.

4. **The market does not currently know Forge exists, and AGPL doesn't change that.** AGPL protects a project people want to take. Forge's present risk is obscurity, not appropriation. The license's real strategic value is different: with all copyright held by one person **[repo: git shortlog — effectively sole authorship]**, dual-licensing and OEM deals stay open as a future lever.

What the market *does* care about right now, with money attached: variable AI spend it can't see or cap, audit trails procurement now demands, and inference plans. Forge's mesh + Store happens to be an unusually good foundation for the first two.

---

## 3. Market findings (researched 2026-07-30)

**Coding-agent spend is huge, concentrated, and shifting to usage billing.**
- Cursor: ~$1B ARR Nov 2025, ~$2B by early 2026, with reports of ~$4B annualized by mid-2026 (later figures less corroborated). Pricing: Pro $20, Pro+ $60, Ultra $200; Teams $40/seat (Premium $120); Enterprise custom. ([TNW](https://thenextweb.com/news/cursor-anysphere-2-billion-funding-50-billion-valuation), [Cursor blog](https://cursor.com/blog/teams-pricing-june-2026), [NxCode](https://www.nxcode.io/resources/news/cursor-ai-pricing-plans-guide-2026))
- GitHub Copilot moved to usage-based billing (AI Credits) on June 1, 2026 — every plan now has a metered component; agentic runs draw down credits. ([GitHub blog](https://github.blog/news-insights/company-news/github-copilot-is-moving-to-usage-based-billing/), [unerr](https://www.unerr.dev/blog/github-copilot-pricing-explained))
- Claude Code Enterprise: $20/seat + usage at API rates; real spend $60–$250+/user/mo. ([eesel](https://www.eesel.ai/blog/enterprise-claude-code))
- Consequence: **"agentic bill shock" is now a named, budgeted problem** in orgs of every size.

**Cost governance is where new budget is appearing.**
- FinOps Foundation: AI-spend management among FinOps teams went 31% → 98% in two years; cost-governance tools price at ~0.25–1% of monitored AI spend; governance platforms at $4K–$15K/mo. ([TrueFoundry buyer's guide](https://www.truefoundry.com/blog/enterprise-ai-agent-security-solutions), [Zylos](https://zylos.ai/research/2026-07-02-buyer-side-governance-enterprise-ai-agent-deployments/))
- Enterprise buyers now demand kill switches, evidentiary audit trails, hash-chained logs, per-step tracing tagged with agent ID + model version (SOC2 / EU AI Act). ([miniorange](https://www.miniorange.com/blog/ai-agent-audit-trail/), [Speakeasy](https://www.speakeasy.com/blog/2026-year-of-ai-governance))
- LiteLLM (open-source gateway) monetizes exactly this ladder: free self-host → Enterprise from ~$250/mo to ~$30K/yr for SSO, audit logs, per-team spend tracking, RBAC. ([litellm.ai/pricing](https://www.litellm.ai/pricing), [TrueFoundry](https://www.truefoundry.com/blog/litellm-pricing-guide))

**Routing/aggregation monetizes as infrastructure, not as a client feature.**
- OpenRouter: ~$50M annualized revenue (March 2026) on a ~5–5.5% take rate over inference; $1.3B valuation. ([Sacra](https://sacra.com/c/openrouter/))
- Martian (~$1.3B valuation reported), NotDiamond (free via OpenRouter, enterprise custom) — routing *quality* itself is being given away at the margin; the money is in the payment/aggregation relationship or the enterprise contract. ([Medium/Martian](https://medium.com/@sarawgiapoorvwork347/martian-the-san-francisco-based-startup-that-invented-the-first-llm-router-is-reportedly-nearing-4211dd768296), [NotDiamond](https://github.com/Not-Diamond/awesome-ai-model-routing))

**Open-source coding agents monetize inference or org features — never the tool.**
- OpenCode (~171K stars): free tool; money from **Zen** — curated inference, Go plan $10/mo flat with usage caps, PAYG at zero markup. ([opencode.ai/zen](https://opencode.ai/zen), [Developers Digest](https://www.developersdigest.tech/blog/opencode-developer-guide-2026))
- Cline: ~5M installs Jan 2026, ~$5M ARR Aug 2025 **[est.]**; zero-markup inference passthrough; monetization = team licensing + enterprise governance, converting slowly. ([Sacra](https://sacra.com/c/cline/))
- Aider: no monetization; release cadence collapsed to occasional maintenance releases (v0.86.2, Feb 2026). ([HN](https://news.ycombinator.com/item?id=46067907), [Sem Sinchenko](https://semyonsinchenko.github.io/ssinchenko/post/aider_2026_and_other_topics/))

**Anthropic closed third-party subscription use.**
- OAuth tokens from Free/Pro/Max are restricted to Claude Code and Claude.ai; use in any other tool is a consumer-ToS violation; enforced April 4, 2026, with one-month credits as compensation. ([The Register](https://www.theregister.com/2026/02/20/anthropic_clarifies_ban_third_party_claude_access/), [GIGAZINE](https://gigazine.net/gsc_news/en/20260220-anthropic-third-party-block/))

**Adjacent proven-WTP markets.**
- AI code review: CodeRabbit $24/user/mo, ~750K registered users, ARR estimated $40–60M **[est.]**; Greptile $30/user then $1/review past 50. Proven money, crowded field. ([Levelop](https://levelop.dev/blog/best-ai-code-review-tools-2026-coderabbit-greptile-qodo-compared), [dev.to](https://dev.to/jovan_chan_9500711396d4e6/greptile-review-2026-82-bug-catch-rate-the-1review-trap-and-who-should-pay-30month-4jao))
- Remote agent control: Omnara ~$20/mo (YC S25); Conductor raised $22M (Mac parallel-agent app); Happy is open-source and free; Terragon free beta. Small, contested, and being absorbed by first-party vendors (Claude Code web/mobile, Cursor background agents, Codex cloud). ([Omnara App Store](https://apps.apple.com/us/app/omnara-claude-codex-mobile/id6748426727), [happy.engineering](https://happy.engineering/), [codeongrass](https://codeongrass.com/blog/best-app-to-control-coding-agents-from-mobile/))
- Devin $500/mo teams (Beta $20/mo); Factory $20/mo entry; Amp free ad-supported tier + zero-markup PAYG. ([Tembo](https://www.tembo.io/blog/coding-cli-tools-comparison), [TECHSY](https://techsy.io/en/blog/background-coding-agents-compared))

---

## 4. Competitive map

| Player | Buyer | Paid outcome | Pricing | Traction signal | Exploitable gap |
|---|---|---|---|---|---|
| Cursor | Individual dev → org | Fastest mainstream agentic IDE | $20–$200 ind.; $40–120/seat teams | ~$2B+ ARR | Vendor-priced inference; no BYO-cost control; closed |
| GitHub Copilot | Org (CTO) | Default enterprise assistant | $10–100 ind.; $19–39/seat + credits | Bundled distribution | Usage billing just created org-wide bill anxiety — no cross-tool cost control |
| Claude Code | Individual → org | Best frontier agent | $20–200 subs; Enterprise $20/seat+usage | Category default | Single vendor; org spend opaque without extra tooling |
| OpenCode | Individual dev | Free agent + cheap curated inference (Zen $10/mo) | $0 tool; inference plans | ~171K stars | No org control plane; no audit story; routing is manual |
| Cline | Individual → enterprise | Free tool; team governance licensing | $0 + enterprise | 5M installs, ~$5M ARR [est.] | Proof conversion is slow; VS-Code-bound; no cost routing |
| Aider | Individual | — (never monetized) | $0 | Maintenance mode | Cautionary tale, not a competitor |
| OpenRouter | App developers | One API, 5% take | 5–5.5% of spend | ~$50M rev [est.] | Doesn't touch the coding-agent loop or org policy |
| LiteLLM | Platform eng | Self-host gateway + enterprise controls | $250/mo→$30K/yr | De-facto OSS gateway | Gateway sees requests, not agent *behavior*; no provenance/outcome data |
| CodeRabbit / Greptile | Eng manager | Bugs caught in PRs | $24–30/user | $40–60M ARR [est.] | Entrenched; entering now as a solo player is a distribution fight |
| Omnara / Conductor / Happy | Individual dev | Remote/parallel agent control | $20/mo / free | $22M raised (Conductor); Happy free OSS | First-party absorption underway; thin, contested niche |

**The open cell:** nobody sits at the intersection of *coding-agent harness* × *cross-provider cost routing* × *org-level policy/audit*. LiteLLM governs API calls but is blind to agent behavior and outcomes. Copilot/Cursor govern only their own walled garden. Cline has governance ambitions but no cost-routing engine. Forge's mesh + Store + blame is, technically, already most of that product **[repo]** — what's missing is the org altitude (central policy, aggregation, SSO) and, critically, users.

---

## 5. Candidate ranking

Fourteen candidates generated; scored 1–10 on twelve criteria (§ methodology: customer pain, willingness/ability to pay, reachable market, Forge-specific advantage, differentiation/defensibility, distribution feasibility, speed to validated revenue, gross margin, operational burden [inverted], vendor dependence [inverted], open-source fit, solo-execution realism). Scores are my calibrated estimates from the evidence above — treat the *ordering* as more reliable than the absolute numbers.

| # | Candidate | Pain | WTP | Reach | Forge adv. | Defens. | Distrib. | Speed | Margin | Ops | Vendor-indep. | OSS fit | Solo | **Total /120** |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | **Org Control Plane (open-core: budgets, policy, audit, SSO)** | 8 | 8 | 6 | 8 | 7 | 4 | 4 | 9 | 6 | 8 | 9 | 6 | **83** |
| 2 | **Forge Anywhere Pro (hosted relay + push + tunnel convenience)** | 5 | 6 | 4 | 7 | 4 | 5 | 9 | 8 | 7 | 6 | 8 | 9 | **78** |
| 3 | OEM / dual-license the harness+mesh engine (AGPL forcing function) | 6 | 7 | 3 | 8 | 8 | 3 | 3 | 10 | 8 | 8 | 7 | 6 | **77** |
| 4 | Team cost-savings dashboard only (FinOps-lite, no policy server) | 7 | 6 | 5 | 8 | 5 | 4 | 5 | 9 | 7 | 7 | 8 | 7 | **78→ merged into #1** |
| 5 | Assay-powered PR review GitHub App | 8 | 9 | 7 | 4 | 3 | 3 | 5 | 7 | 5 | 6 | 7 | 5 | **69** |
| 6 | Curated inference plan ("Forge Fuel", à la OpenCode Zen) | 7 | 8 | 5 | 2 | 2 | 3 | 4 | 3 | 4 | 3 | 8 | 4 | **53** |
| 7 | Hosted mesh-as-API (compete with OpenRouter) | 6 | 7 | 4 | 3 | 2 | 2 | 3 | 4 | 3 | 4 | 6 | 3 | **47** |
| 8 | Managed cloud Forge sessions (compete Devin/Codex cloud) | 7 | 7 | 4 | 3 | 2 | 2 | 2 | 3 | 2 | 3 | 5 | 2 | **42** |
| 9 | Model/agent benchmarking-as-a-service (`forge bench` productized) | 5 | 5 | 3 | 7 | 5 | 3 | 4 | 7 | 5 | 7 | 8 | 6 | **65** |
| 10 | Skills/workflow marketplace | 3 | 3 | 2 | 4 | 4 | 2 | 2 | 8 | 5 | 6 | 6 | 4 | **49** |
| 11 | Sponsorware / donations / paid support | 2 | 2 | 2 | 5 | 2 | 5 | 8 | 9 | 8 | 9 | 10 | 9 | **71\*** |
| 12 | Vertical productized service (legacy-modernization audits via Lattice+Assay) | 7 | 8 | 3 | 6 | 3 | 4 | 8 | 4 | 3 | 8 | 6 | 5 | **65** |
| 13 | Per-outcome pricing (pay per resolved issue) | 6 | 5 | 3 | 5 | 4 | 2 | 2 | 4 | 3 | 4 | 6 | 3 | **47** |
| 14 | Desktop/mobile app as paid product (free CLI) | 4 | 4 | 4 | 5 | 3 | 4 | 6 | 8 | 6 | 7 | 4 | 7 | **62** |

\* #11 scores high on feasibility but fails the brief's bar ("meaningful revenue, not incidental donations") — excluded from finalists on ceiling, not score.

**Why the big categories lose:**
- **Inference resale (#6, #7):** zero Forge advantage, thin/negative margins without volume, OpenCode Zen and OpenRouter already own it with 4–5 orders of magnitude more distribution. A model vendor or aggregator absorbs any success instantly.
- **Managed execution (#8):** capital-intensive, competes directly with the model vendors' own subsidized cloud agents (Codex cloud, Cursor background agents, Devin). Structurally advantaged incumbents.
- **PR review (#5):** best proven WTP in the whole map, but Forge brings no distribution and no unique detection advantage against $40–60M-ARR incumbents; it would mean building a second company unrelated to Forge's surface.
- **Marketplace (#10), outcomes (#13):** require an install base / trust that don't exist.

**Pre-mortem on the two leaders:**
- *Control Plane dies if:* free adoption never materializes (most likely failure — base rate for new terminal agents in 2026 is brutal); or orgs standardize on single-vendor agents (Copilot/Cursor) whose bundled dashboards are "good enough," leaving cross-provider governance a niche of a niche; or LiteLLM/Cline ships agent-level cost policy first. **Disproving evidence to watch:** 90 days of promotion yielding <500 weekly actives; design-partner interviews where eng leads say "our vendor dashboard suffices."
- *Anywhere Pro dies if:* Claude Code's own web/mobile + Happy (free OSS) cover the need; conversion of a tiny user base rounds to zero. **Disproving evidence:** <2% of active users start a trial in 60 days.

---

## 6. Top three deep dives

### Finalist 1 — Forge Control Plane (primary thesis)

- **Product:** `forge-hub` — a self-hosted server (same Rust workspace, license-keyed binary) that Forge agents attach to. Org admin gets: central provider key vault (devs never hold raw keys); per-dev/team/repo budget caps enforced at the mesh; org routing policy (allowed models, credit_mode, data-residency rules e.g. "no code to provider X"); aggregated spend + savings dashboards ("routing saved you $X vs. list price" — computable today from the `routing_decision` table **[repo]**); org-wide `forge blame` provenance; hash-chained audit log export (SIEM/SOC2/EU-AI-Act-shaped); SSO/RBAC (enterprise tier).
- **ICP:** platform/eng-productivity lead, 10–200 devs, multi-tool AI usage, EU-heavy segments where self-hosting + audit are procurement requirements. **User:** developers (unchanged, free). **Buyer:** eng manager / director with a new AI line item.
- **Packaging/pricing:** Team $15/active-dev/mo ($99/mo floor, founding-partner $99 flat); Enterprise $1K–2.5K/mo (SSO/SCIM, audit export, support SLA). Billing unit: active developer, because value scales with people spending money, and seats are the unit every anchor product trained buyers on.
- **GTM:** the free agent's cost story is the funnel ("the agent that cuts your AI bill"), content comparing real per-task cost across providers (Forge uniquely generates this data), then design-partner sales.
- **Economics:** self-hosted → near-zero infra COGS; margin is support time. Renewal driver: the monthly savings + audit report makes the product self-justifying at renewal.
- **Moat:** the audit/provenance data model is deep in Forge's Store; a gateway (LiteLLM) can't see agent behavior, a single-vendor tool won't cover rivals. AGPL + sole copyright keeps a relicensing lever.
- **Dependencies/failure:** requires free-tier adoption first (the gate in §8); requires 2–3 org features (SSO, aggregation) that don't exist yet; fails if bundled vendor dashboards are "good enough."

### Finalist 2 — Forge Anywhere Pro (bridge revenue, ship first)

- **Product:** paid tier on the existing app: hosted APNs push relay (exists **[repo: forge-relay]**), hosted E2E relay w/ blob offload (exists), zero-config tunnel, multi-device, priority OTA channel. Free tier keeps full self-hosted Anywhere (protocol is open; AGPL demands honesty here — see `docs/anywhere/agpl-service-boundary-checklist.md` **[repo]**).
- **Price:** $6–8/mo (undercut Omnara's $20; Happy is free, so the paid delta is *hosted convenience + push + polish*, not capability).
- **Why ship it despite the low ceiling:** it's ~2 weeks of gating work on infrastructure that already runs; it produces the project's first WTP data and first renewals; it funds hosting. **It is a supporting feature, not the business** — expected outcome is hundreds, not thousands, of subscribers even with healthy adoption.
- **Failure mode:** first-party vendor apps + free OSS competitors make hosted convenience worth $0 to most; churn if push reliability slips (single-operator relay).

### Finalist 3 — OEM / commercial licensing of the harness engine

- **Product:** commercial (non-AGPL) license of `forge-core`/`forge-mesh`/`forge-store` for companies embedding an agent loop in their own product and unwilling to open their source; optionally with paid integration support. Sole copyright makes this legally clean **[repo: authorship]**.
- **ICP/buyer:** devtool and vertical-SaaS companies adding agentic features; buyer is their CTO. Pricing: $10K–$50K/yr per product + support.
- **Honest assessment:** highest margin, real defensibility (the AGPL *is* the sales agent), but demand is speculative — zero inbound today, and the sales motion is slow and lumpy for a solo founder. Keep the door open (license headers, a LICENSING.md, a "commercial licensing" email), spend no build effort on it. It becomes real only after Forge is known.

---

## 7. Winning business design

- **Free forever (explicit):** the entire single-developer experience — agent, TUI, mesh routing, failover, Lattice, skills, workflows, voice, desktop/mobile apps, self-hosted `forge serve` + self-hosted Anywhere relay, `forge blame`, local spend view. This is non-negotiable: the free tier is the only acquisition channel Forge has.
- **Paid boundary:** anything *organizational* (shared policy, central keys, aggregation across developers, SSO/RBAC, audit export, hosted convenience). The boundary is legible, defensible, and matches LiteLLM/Grafana precedent: individuals never hit it; companies hit it the day a manager asks "what are we spending?"
- **Initial SKUs:**
  1. **Anywhere Pro** — $6/mo or $60/yr, per user. Ships first (infra exists).
  2. **Control Plane Team** — $15/active dev/mo, $99/mo floor; founding partners $99/mo flat for 12 months.
  3. **Control Plane Enterprise** — from $1K/mo, annual, invoice; SSO/SCIM, audit export, SLA.
- **Billing unit:** per active developer (Control Plane); per user (Anywhere Pro). Not per token (LiteLLM's lesson), not per host, not per outcome.
- **Infra/support costs:** Anywhere Pro: one small VM + APNs — tens of $/mo until thousands of users. Control Plane: customer-hosted, so COGS ≈ support hours; the solo-founder constraint caps how many enterprise logos are serviceable — price accordingly (higher, fewer).
- **Channel:** GitHub/HN/Reddit for the free agent; the mesh cost-savings story is the shareable wedge ("same model, 21% cheaper per fix" is already benchmarked **[repo]**). Control Plane sold founder-led to design partners recruited from free-tier telemetry-free signups (opt-in "team interest" flag).
- **Renewal logic:** monthly savings-vs-list-price report + audit exports procurement already filed — removing the product recreates the visibility gap.
- **Expansion:** seats grow with AI adoption; Enterprise tier upsell; later, policy for *non-Forge* agents (govern Claude Code/Codex sessions through Forge's bridge machinery) — which would widen the market from "Forge shops" to "any shop," and is the long-term prize.

---

## 8. Validation before building (30/60/90)

**Phase 0 gate (days 1–30): distribution, not monetization.**
- Launch properly: Show HN, r/LocalLLaMA, r/ChatGPTCoding, lobste.rs, X — the cost-routing story, with reproducible benchmark receipts.
- Instrument (privacy-respecting, opt-in) install/active counts; add a "Forge for Teams — join the design program" page with pricing shown ($15/dev) and a 2-field form.
- 15 interviews with eng leads/platform engineers on agent spend visibility (recruit via the form + own network).
- **Success:** ≥1,000 stars or ≥500 weekly actives; ≥10 team-page signups. **Failure:** <150 stars and <50 actives after a real launch push → the agent isn't landing; do not proceed to paid work; diagnose positioning first.

**Phase 1 (days 30–60): concierge + pre-sell.**
- Ship Anywhere Pro billing (Stripe + license key) — measure trial-start rate.
- Recruit 3–5 Control Plane design partners; deliver *concierge*: manual weekly spend/savings report generated from their Forge Stores, hand-configured org policy files. Charge founding price ($99/mo) from day one — unpaid pilots are not validation.
- **Success:** ≥3 paying design partners, Anywhere Pro trial-start ≥2% of actives. **Kill:** zero orgs willing to pay $99/mo even concierge-grade → the org thesis is wrong; fall back to OEM/services or park monetization and keep building adoption.

**Phase 2 (days 60–90): build only what's paid for.**
- Automate whatever the concierge phase did by hand, in priority order of partner demand.
- **Success:** 3+ partners renewed month 2; a pipeline of ≥10 further teams. **Kill criteria (hard):** by day 90, if MRR < $500 *and* weekly actives < 500, stop monetization work entirely for two quarters and treat Forge as a distribution project.

---

## 9. Minimum monetizable product

**Must build (first paying customers):**
1. License-key check + Stripe billing (shared by both SKUs).
2. Anywhere Pro gating: hosted-relay pairing behind entitlement; free path = self-host (docs already exist **[repo]**).
3. Control Plane concierge tooling: a script that ingests a team's Forge Store DBs and emits the spend/savings/provenance report (most of the data model exists **[repo]**), plus a served org policy file (`forge-config` already merges layered config **[repo]**).
4. A pricing page and a LICENSING.md (commercial-license contact).

**Attractive distractions (explicitly deferred):** SSO/SCIM (until an enterprise deal demands it), hosted dashboard UI (concierge reports first), marketplace, mesh-as-API, per-outcome billing, any new agent capability justified as "for monetization."

---

## 10. Confidence and disconfirming evidence

- **Confidence that the free/paid boundary (individual free, org paid) is right: 80%.** Strongest evidence: every comparable OSS devtool that monetized durably (LiteLLM, Grafana-pattern, Cline's direction, GitLab) converged here; every counter-pattern (paid individual harness) has no surviving example.
- **Confidence that the Control Plane is the right first paid product (vs. PR-review, inference, hosting): 60%.** It rests on one solid external trend (AI-spend governance budgets) and one honest internal fact (the data model exists), but on an **unvalidated** assumption that multi-provider governance is urgent for orgs of Forge's reachable size — bundled vendor dashboards being "good enough" is the live alternative explanation.
- **Confidence Forge reaches the adoption gate at all: 25–35%.** This is the weakest link and it is not a monetization variable. The 2026 terminal-agent field is late, crowded, and vendor-subsidized; 8 stars after 6 weeks is the base rate speaking. The mesh cost story is a genuine wedge, but wedges need swings: launch effort so far is the untested variable.
- **What would change the recommendation:**
  - Interviews revealing orgs *won't* self-host but *will* pay for a hosted governance layer → flip Control Plane to hosted (raises COGS, changes AGPL posture).
  - A surge of individual Anywhere Pro conversions (>5% of actives) → individual convenience is worth more than modeled; raise its priority and price.
  - Inbound OEM interest → licensing jumps the queue (it's pure margin).
  - Anthropic/OpenAI shipping org-level *cross-tool* spend governance → the open cell closes; retreat to EU/self-host/audit niche or OEM.
  - Adoption failing the Phase 0 gate → all monetization is moot; the correct spend of the next two quarters becomes distribution and positioning, and this document's §7–9 go in a drawer, not in the roadmap.
