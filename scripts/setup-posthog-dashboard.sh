#!/usr/bin/env bash
# Create/update the curated Forge analytics dashboard. The personal API key is used only for this
# control-plane setup and is never stored; the public project token remains the ingestion key.
set -euo pipefail

: "${POSTHOG_PERSONAL_API_KEY:?set a scoped PostHog personal API key}"
: "${POSTHOG_PROJECT_ID:?set the PostHog project id}"

host="${POSTHOG_API_HOST:-https://eu.posthog.com}"
auth=(-H "Authorization: Bearer ${POSTHOG_PERSONAL_API_KEY}")
json=(-H "Content-Type: application/json")
dashboard_api="${host%/}/api/projects/${POSTHOG_PROJECT_ID}/dashboards/"
insight_api="${host%/}/api/projects/${POSTHOG_PROJECT_ID}/insights/"

existing="$(curl --fail --silent --show-error "${auth[@]}" \
  --get --data-urlencode "search=Forge health" "$dashboard_api")"
dashboard_id="$(printf '%s' "$existing" |
  jq -r '.results[] | select(.name == "Forge health") | .id' | head -1)"

if [ -z "$dashboard_id" ]; then
  dashboard_id="$(curl --fail --silent --show-error -X POST "${auth[@]}" "${json[@]}" \
    --data-binary '{
      "name": "Forge health",
      "description": "Anonymous Forge adoption, activity, reliability, feature usage, and public distribution counts. Charts use total events because Forge transmits no stable user identifier.",
      "pinned": true,
      "tags": ["forge", "anonymous-telemetry"]
    }' "$dashboard_api" | jq -r '.id')"
fi

create_insight() {
  local name="$1" description="$2" query="$3" results id payload
  results="$(curl --fail --silent --show-error "${auth[@]}" \
    --get --data-urlencode "search=${name}" "$insight_api")"
  id="$(printf '%s' "$results" |
    jq -r --arg name "$name" '.results[] | select(.name == $name) | .id' | head -1)"
  payload="$(jq -n \
    --arg name "$name" \
    --arg description "$description" \
    --argjson dashboard "$dashboard_id" \
    --argjson query "$query" \
    '{name: $name, description: $description, dashboards: [$dashboard], query: $query}')"
  if [ -z "$id" ]; then
    id="$(curl --fail --silent --show-error -X POST "${auth[@]}" "${json[@]}" \
      --data-binary "$payload" "$insight_api" | jq -r '.id')"
  else
    curl --fail --silent --show-error -X PATCH "${auth[@]}" "${json[@]}" \
      --data-binary "$payload" "${insight_api}${id}/" >/dev/null
  fi
  printf '%s\t%s\n' "$id" "$name"
}

trend_query() {
  local event="$1" label="$2" date_from="$3" interval="$4" breakdown="$5" display="$6"
  jq -n \
    --arg event "$event" \
    --arg label "$label" \
    --arg date_from "$date_from" \
    --arg interval "$interval" \
    --arg breakdown "$breakdown" \
    --arg display "$display" \
    '{
      kind: "InsightVizNode",
      source: {
        kind: "TrendsQuery",
        series: [{kind: "EventsNode", event: $event, math: "total", custom_name: $label}],
        dateRange: {date_from: $date_from},
        interval: $interval,
        breakdownFilter: {breakdown: $breakdown, breakdown_type: "event", breakdown_limit: 12},
        trendsFilter: {display: $display, showValuesOnSeries: true}
      }
    }'
}

create_insight \
  "Active now (30-minute windows)" \
  "Foreground installations seen in the latest anonymous activity windows." \
  "$(trend_query forge_active_window "Active installations" -1h hour surface ActionsBarValue)"

create_insight \
  "Daily active installations" \
  "One locally deduplicated event per active installation and UTC day." \
  "$(trend_query forge_active_day "Daily active" -30d day surface ActionsLineGraph)"

create_insight \
  "Weekly active installations" \
  "One locally deduplicated event per active installation and ISO week." \
  "$(trend_query forge_active_week "Weekly active" -12w week surface ActionsLineGraph)"

create_insight \
  "Monthly active installations" \
  "One locally deduplicated event per active installation and UTC month." \
  "$(trend_query forge_active_month "Monthly active" -12m month surface ActionsLineGraph)"

create_insight \
  "New installations" \
  "First telemetry-enabled release launch; an install signal rather than a storefront guarantee." \
  "$(trend_query forge_installed "New installations" -90d day surface ActionsStackedBar)"

create_insight \
  "First successful activation" \
  "First successful CLI/TUI run or desktop/mobile server pairing." \
  "$(trend_query forge_activated "Activated" -90d day surface ActionsStackedBar)"

create_insight \
  "Version adoption (last 7 days)" \
  "Active installation-days broken down by released Forge version." \
  "$(trend_query forge_active_day "Version" -7d day version ActionsPie)"

reliability_query="$(jq -n '{
  kind: "InsightVizNode",
  source: {
    kind: "TrendsQuery",
    series: [
      {kind: "EventsNode", event: "forge_run_succeeded", math: "total", custom_name: "Succeeded"},
      {kind: "EventsNode", event: "forge_run_failed", math: "total", custom_name: "Failed"}
    ],
    dateRange: {date_from: "-30d"},
    interval: "day",
    trendsFilter: {display: "ActionsLineGraph", showValuesOnSeries: true}
  }
}')"
create_insight \
  "Run reliability" \
  "Completed CLI/TUI runs by outcome. No error text or session identifier is collected." \
  "$reliability_query"

feature_query="$(jq -n '
  [
    ["forge_feature_mesh", "Mesh"],
    ["forge_feature_voice", "Voice"],
    ["forge_feature_remote", "Remote"],
    ["forge_feature_mcp", "MCP"],
    ["forge_feature_lattice", "Lattice"],
    ["forge_feature_assay", "Assay"],
    ["forge_feature_bench", "Bench"],
    ["forge_feature_automation", "Automation"],
    ["forge_feature_extensibility", "Plugins & skills"]
  ] as $features |
  {
    kind: "InsightVizNode",
    source: {
      kind: "TrendsQuery",
      series: ($features | map({kind: "EventsNode", event: .[0], math: "total", custom_name: .[1]})),
      dateRange: {date_from: "-30d"},
      interval: "day",
      trendsFilter: {display: "ActionsBarValue", showValuesOnSeries: true}
    }
  }')"
create_insight \
  "Feature adoption (30 days)" \
  "Counts of explicit fixed-schema feature commands; no command arguments are collected." \
  "$feature_query"

download_query="$(jq -n --arg query "
  SELECT
    max(toInt(properties.release_downloads_total)) AS all_downloads,
    max(toInt(properties.cli_downloads_total)) AS cli_downloads,
    max(toInt(properties.desktop_downloads_total)) AS desktop_downloads,
    max(toInt(properties.mobile_downloads_total)) AS mobile_downloads,
    argMax(properties.latest_tag, timestamp) AS latest_tag
  FROM events
  WHERE event = 'forge_distribution_snapshot'
" '{
  kind: "DataTableNode",
  source: {kind: "HogQLQuery", query: $query},
  full: true,
  showActions: true,
  showDateRange: false,
  showExport: true,
  showReload: true
}')"
create_insight \
  "Public release downloads" \
  "Latest cumulative GitHub release-asset download totals, refreshed daily." \
  "$download_query"

echo "dashboard ready: ${host%/}/project/${POSTHOG_PROJECT_ID}/dashboard/${dashboard_id}"
