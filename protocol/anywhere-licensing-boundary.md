# Forge Anywhere licensing boundary

The Forge repository remains AGPL-3.0-only. Its Anywhere protocol, cryptography, fixtures, CLI host
connector, sync client, capsule code, and mobile/web clients are public here. Local/LAN remote
control, direct pairing, and user-managed `serve --anywhere` tunnels remain free and unchanged.

The separately deployed `forge-anywhere-service` is private. It independently implements the
published wire/API contract and copies the golden fixture data for compatibility tests. It must not
import, link, vendor, or copy implementation code from any Forge crate. The existing APNs-only
`forge-relay` remains a separate public service and is not expanded into the Anywhere backend.

