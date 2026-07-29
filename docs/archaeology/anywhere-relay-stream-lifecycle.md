# Code archaeology: Anywhere relay stream lifecycle

## Boundary

`connector/streams.rs` owns the per-relay-connection bridge between controller WebSocket frames and one local Forge session socket. The parent connector owns authenticated envelope dispatch; the stream module owns only local socket creation, bidirectional forwarding, stream ownership, and stale-stream closure.

## Invariants

- Stream IDs are unique within a relay connection and remain bound to their originating controller device.
- Only validated GET WebSocket-open bridge requests create local sockets.
- Local socket closure emits one `Closed` event so the parent removes its handle.
- A data frame for a stream forgotten after relay reconnect receives a host-to-controller close; a controller close receives no redundant answer.
- No stream state survives a relay connection; reconnect recovery remains in the parent connector.

## Characterization

`a_frame_for_a_forgotten_stream_closes_that_stream_instead_of_the_connector` protects the stale-stream recovery contract. Connector integration tests cover route validation and encrypted relay dispatch around this boundary.
