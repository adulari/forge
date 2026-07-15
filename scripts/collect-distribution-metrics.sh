#!/usr/bin/env bash
# Send one anonymous aggregate snapshot of public Forge release downloads to PostHog.
set -euo pipefail

: "${POSTHOG_PROJECT_KEY:?set POSTHOG_PROJECT_KEY to the public project token}"

repo="${GITHUB_REPOSITORY:-adulari/forge}"
host="${POSTHOG_HOST:-https://eu.i.posthog.com}"
api="https://api.github.com/repos/${repo}/releases"
releases="$(mktemp)"
trap 'rm -f "$releases" "$releases.page"' EXIT
printf '[]' > "$releases"

page=1
while :; do
  curl --fail --silent --show-error --location \
    -H "Accept: application/vnd.github+json" \
    ${GITHUB_TOKEN:+-H "Authorization: Bearer ${GITHUB_TOKEN}"} \
    "${api}?per_page=100&page=${page}" > "$releases.page"
  count="$(jq 'length' "$releases.page")"
  [ "$count" -eq 0 ] && break
  jq -s '.[0] + .[1]' "$releases" "$releases.page" > "${releases}.next"
  mv "${releases}.next" "$releases"
  [ "$count" -lt 100 ] && break
  page=$((page + 1))
done

properties="$(jq -c '
  [.[].assets[]] as $assets |
  {
    distinct_id: "forge-distribution-aggregate",
    "$process_person_profile": false,
    "$geoip_disable": true,
    schema: 1,
    snapshot_day: (now | strftime("%Y-%m-%d")),
    releases: length,
    release_downloads_total: ($assets | map(.download_count) | add // 0),
    cli_downloads_total: ($assets | map(select(.name | test("^forge-(x86_64|aarch64).*(tar\\.gz|zip)$")) | .download_count) | add // 0),
    desktop_downloads_total: ($assets | map(select(.name | startswith("Forge-desktop-")) | .download_count) | add // 0),
    mobile_downloads_total: ($assets | map(select(.name | endswith(".ipa")) | .download_count) | add // 0),
    latest_tag: (map(select(.draft == false and .prerelease == false)) | sort_by(.published_at) | last | .tag_name // "none")
  }
' "$releases")"

jq -n \
  --arg key "$POSTHOG_PROJECT_KEY" \
  --argjson properties "$properties" \
  '{api_key: $key, event: "forge_distribution_snapshot", properties: $properties}' |
  curl --fail --silent --show-error \
    -H "Content-Type: application/json" \
    --data-binary @- \
    "${host%/}/capture/" >/dev/null

echo "sent anonymous distribution snapshot for ${repo}"
