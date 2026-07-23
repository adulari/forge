# Anonymous usage statistics

Forge sends a few anonymous counters by default so maintainers can see whether releases and
product surfaces are healthy. It uses no analytics SDK, cookie, advertising identifier, account,
installation identifier, device fingerprint, or person profile. All events use the same constant
`distinct_id` (`forge-anonymous`), disable GeoIP enrichment, and are stored in PostHog's EU region.

Disable reporting at any time:

```toml
[telemetry]
enabled = false
```

`DO_NOT_TRACK=1` and `FORGE_TELEMETRY=0` also disable CLI/TUI reporting. Desktop and mobile expose
the same switch under **Settings → Privacy → Anonymous usage statistics**. Opting out deletes any
locally queued counters and does not change product functionality.

## Exact event contract

| Event | Maximum frequency per installation | Meaning |
|---|---:|---|
| `forge_installed` | Once | First telemetry-enabled release launch |
| `forge_active_month` | Once per UTC month | Monthly active installation |
| `forge_active_week` | Once per ISO week | Weekly active installation |
| `forge_active_day` | Once per UTC day | Daily active installation |
| `forge_active_window` | Once per UTC 30-minute window | Recently active installation |
| `forge_run_succeeded` / `forge_run_failed` | Once per completed CLI/TUI run | Run reliability |
| `forge_activated` | Once | First successful CLI/TUI run or app pairing |
| `forge_app_error` | Once per caught root render failure | App-shell render reliability |
| `forge_feature_*` | Once per explicit feature command | Anonymous feature adoption |
| `forge_distribution_snapshot` | Once daily, from GitHub Actions | Aggregate public release downloads |

Application events contain only `surface`, released app `version`, operating-system family,
architecture where available, distribution label, coarse period, and schema version. Distribution
snapshots contain only public GitHub release/download totals and the latest public release tag.
`forge_app_error` additionally contains one closed `error_code` value (`react_render`); its API
cannot accept an error object, message, stack, route, or arbitrary property.

Forge never sends prompts, responses, commands, repository names, filenames, paths, session IDs,
provider/model choices, API keys, account data, error messages, IP-derived location, or arbitrary
caller-provided properties. A local JSON file stores only period markers and unsent event names.
Debug/test builds are disabled unless a developer explicitly sets `FORGE_TELEMETRY_FORCE=1` or
`EXPO_PUBLIC_FORGE_TELEMETRY_FORCE=1`.

These counters measure active **installations**, not people: one person using three devices counts
three times, and clearing application data can count as a new installation. GitHub asset downloads
are downloads rather than guaranteed completed installations. Because a client ingestion token is
necessarily public, a determined third party can spoof analytics events; these figures are product
signals rather than an auditable billing or security ledger.

## Maintainer setup

1. Create a free project at <https://eu.posthog.com> and copy its **Project API key** (`phc_…`).
   Select the EU region; a personal API key is not needed by Forge.
2. From a checkout authenticated with GitHub CLI, run:

   ```bash
   gh variable set POSTHOG_PROJECT_KEY --body 'phc_REPLACE_ME'
   gh variable set POSTHOG_HOST --body 'https://eu.i.posthog.com'
   gh workflow run anonymous-telemetry.yml
   ```

   The project token is designed to be public in client applications, so repository **variables**
   are appropriate. It grants event ingestion only and cannot read analytics or modify the project.
3. For production EAS/Xcode Cloud builds that do not run through GitHub Actions, add these build
   environment variables as plain text:

   ```text
   EXPO_PUBLIC_POSTHOG_KEY=phc_REPLACE_ME
   EXPO_PUBLIC_POSTHOG_HOST=https://eu.i.posthog.com
   ```

   GitHub CLI/TUI releases, GitHub desktop builds, SideStore builds, and OTA updates are already
   wired to the repository variables automatically.
4. Ship one release (or OTA update for mobile). Local debug builds intentionally emit nothing.

### Dashboard

Create or repair the complete dashboard with a temporary personal API key scoped to
`dashboard:read`, `dashboard:write`, `insight:read`, and `insight:write`:

```bash
POSTHOG_PERSONAL_API_KEY=phx_REPLACE_ME \
POSTHOG_PROJECT_ID=225009 \
scripts/setup-posthog-dashboard.sh
```

The script is idempotent and does not store the personal key. Revoke the key after setup.

The generated **Forge health** dashboard contains these trends. Each event is already
client-deduplicated, so every chart uses **Total events**, not unique users:

| Insight | Event/query | Breakdown |
|---|---|---|
| Active now | `forge_active_window`, last 30 minutes | `surface` |
| Daily active installations | `forge_active_day` | `surface` |
| Weekly active installations | `forge_active_week` | `surface` |
| Monthly active installations | `forge_active_month` | `surface` |
| New activated installations | `forge_installed` | `surface` |
| Version adoption | `forge_active_month` | `version` |
| First successful run | `forge_activated` | `surface` |
| Run reliability | Formula: `forge_run_succeeded / (succeeded + failed)` | `surface` |
| App render failures | `forge_app_error` | `version` (filter by `surface`, `error_code`) |
| Feature adoption | All `forge_feature_*` events | Event name |

For downloads, create a **SQL** insight using the latest aggregate snapshot:

```sql
SELECT
  max(properties.release_downloads_total) AS all_downloads,
  max(properties.cli_downloads_total) AS cli_downloads,
  max(properties.desktop_downloads_total) AS desktop_downloads,
  max(properties.mobile_downloads_total) AS mobile_downloads
FROM events
WHERE event = 'forge_distribution_snapshot'
```

PostHog's free tier currently includes one million events per month and one year of retention. An
installation kept continuously active can emit at most about 1,476 activity counters in a 30-day
month (48 half-hour windows per day plus daily, weekly, and monthly counters); normal interactive
usage is substantially lower.
