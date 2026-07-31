# Diagnostics and update coordination

Forge separates passive operational visibility from the deeper, active checks run by
`forge doctor`.

The companion app's **Diagnostics & Updates** screen works over Direct connections and Forge
Anywhere. It shows:

- the installed app and daemon versions plus their remote protocol versions;
- an explicit compatibility verdict and which side must update when protocols differ;
- daemon platform, process uptime, PID, aggregate session/terminal activity, and push readiness;
- bounded process/system memory, CPU count, and load averages;
- safe checks for the session database, terminal runtime, layered configuration, and Git;
- one shared signed-desktop-updater state used by the launch check, root notice, Settings, and the
  diagnostics screen.

Equal protocol versions are compatible even when release versions differ. If the daemon protocol
is older, run `forge update` on the host and restart `forge serve`. If the client protocol is older,
update the installed app. A protocol mismatch takes priority over an ordinary available-update
notice.

## Security boundary

`GET /api/diagnostics` is authenticated under the same token path as every other daemon API.
Forge Anywhere can invoke it only through the explicit, read-only `Diagnostics` bridge route; the
connector still cannot proxy arbitrary paths or methods.

The response contains aggregate operational facts only. It never contains:

- daemon tokens or connection URLs;
- provider credentials or environment values;
- workspace/repository paths or file contents;
- prompts, responses, transcripts, or tool output;
- log paths, log contents, or trace payloads.

The **Copy sanitized summary** action is stricter than the display. It copies only fixed numeric
fields, versions/protocols, platform identifiers, updater state, and statuses for a fixed set of
known checks. It excludes the host name and all daemon-provided free text.

## Why this does not replace `forge doctor`

Passive app diagnostics must remain quick, bounded, and safe to request remotely. `forge doctor`
performs active provider, credential, bridge, terminal-mode, Git, configuration, and live
connectivity probes on the host. Run it when a displayed check warns or when the app cannot explain
a host-side failure. The diagnostics screen intentionally does not claim it can open remote logs.
