# Browser control and network inspection

Forge can drive a real Chrome and read everything that browser fetches — the same data the DevTools
Network tab shows.

`web_fetch` retrieves a URL with an HTTP client. That is the wrong tool for anything behind a login,
anything rendered by JavaScript, and anything whose *traffic* is the thing under investigation. This
feature covers that gap.

## Tools

Two tools, not a dozen: every tool schema sits in the system prompt of every turn, so the surface is
deliberately small.

### `browser`

`action` is one of `open`, `navigate`, `click`, `type`, `eval`, `html`, `screenshot`, `cookies`,
`close`.

| argument | meaning |
| --- | --- |
| `url` | for `open` / `navigate` |
| `selector` | CSS selector, for `click` / `type` |
| `text` | text to type |
| `expression` | JavaScript, for `eval` |
| `headless` | open without a window. Default `false` |
| `profile` | named persistent profile. Default `"default"` |
| `max_chars` | cap for `html` output |

### `browser_network`

`action` is one of `list`, `body`, `clear`.

| argument | meaning |
| --- | --- |
| `url_contains` | case-insensitive URL filter |
| `method` | `GET`, `POST`, … |
| `resource_type` | `Document`, `XHR`, `Fetch`, `Script`, `Preflight`, … |
| `status_min` / `status_max` | inclusive status range; either may be given alone |
| `with_post_data` | only requests that carried a body |
| `limit` | newest N matches. Default 50 |
| `request_id` | for `body` — the id from `list` |

`list` returns a one-line index per request; bodies and headers are fetched by id, so a busy page
does not flood the context.

## Windowed or headless

Windowed is the default and is usually what you want. A headless browser is trivially fingerprinted
and is blocked by many of the sites worth investigating. Pass `headless: true` for CI, a headless
host, or bulk work nobody is watching.

## The profile

Chrome **refuses** remote debugging against its default profile directory:

```
DevTools remote debugging requires a non-default data directory. Specify this using --user-data-dir.
```

So Forge cannot attach to the exact Chrome profile you browse with. Instead it owns a persistent
profile per name under `$XDG_DATA_HOME/forge/browser/<profile>` (override the root with
`FORGE_BROWSER_PROFILE_ROOT`). It is a real Chrome in every other respect, and because the profile
persists:

1. `browser` with `action: "open"` launches a window.
2. You log in by hand, once, in that window.
3. Every later turn re-attaches to the same browser and drives the session you left open.

Use different `profile` names to keep separate accounts apart.

## Why not hook `fetch`

The common approach — inject JavaScript that wraps `window.fetch` and `XMLHttpRequest` — sees only
what page script routed through those two APIs. It misses:

- the document load and every subresource
- redirect chains and their `Location` headers
- CORS preflights
- `navigator.sendBeacon`, `EventSource`, WebSocket upgrades
- service-worker traffic
- anything issued before the hook installed

When reverse-engineering an auth flow, those are usually the requests that matter. Forge reads CDP's
`Network` domain, which is the source DevTools itself renders.

## Response bodies

Bodies live in the renderer, not in Forge's log. Chrome evicts them as the page runs, so:

- asking for a body before the response finished reports exactly that, rather than returning an
  empty string that would read as "the server returned nothing";
- asking long after the fact can fail because Chrome no longer holds it. Capture and read close to
  the request, or `clear` the log and repeat the interaction.

Binary bodies are reported as their size and MIME type rather than dumped.

## Permissions

Both tools declare `SideEffect::Network`, so they cross the permission broker like any other egress.
`eval` runs arbitrary script in a page that may be logged into your accounts — that is the intended
capability, and the reason it is gated rather than treated as a read.

## Testing

Unit tests cover the protocol framing, the network capture (including redirect chains, numeric
headers, failures, bounds, and filters), the launch arguments, and the rendered index — none of
which need a browser.

The end-to-end tests need a real Chrome and are `#[ignore]`d so their absence on a CI runner cannot
look like a pass:

```bash
cargo test -p forge-agent-browser --test live_chrome -- --ignored --nocapture
```

They serve a fixture page from a local socket, so they need no network and cannot break because a
third-party site changed.
