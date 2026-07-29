# Code archaeology: Codex stale response-chain recovery

## Boundary

The session-scoped Codex WebSocket chain retains `previous_response_id` only for incremental continuations. `codex_websocket::is_stale_previous_response_error` is the narrow recognition boundary for backend errors that explicitly invalidate that ID; `CodexOauthProvider::execute_turn_websocket` owns the one retry.

## Invariants

- Only explicit `previous_response_not_found`, `Previous response with id … not found`, or `invalid previous_response_id` errors reset the chain.
- Recovery reconnects exactly once on the same model, account, token, and turn-state route.
- The replacement socket begins with no incremental history, so it resends the full logical request without `previous_response_id`.
- Generic invalid/not-found request errors and all other provider failures are not retried by this path.
- A second failure is returned normally; this recovery cannot loop.

## Characterization

`stale_previous_response_error_matches_only_explicit_chain_rejections` protects the classifier boundary. Existing turn-WebSocket reconnect tests cover the retained-chain transport path; the stale-ID path uses the same bounded reconnect and full-request reset branch.
