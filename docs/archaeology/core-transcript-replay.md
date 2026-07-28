# Code archaeology: Core transcript replay

`messages_to_replay_items` is the one projection from the durable transcript to
surface-independent `ReplayItem`s. Both `Session::replay_items` and
`replay_items_full` call it; duplicating it would let live and persisted replay
drift. Its message visibility filtering is intentional: per-turn system
prompts are machinery, while the compaction marker is the only system message
that represents prior conversation to users.

Tool-result names must be recovered from prior assistant tool calls by ID, and
only the first result line is safe for replay summary. The extraction keeps this
mapping and its ordering unchanged behind a private module. Existing focused
replay and long-session compaction/endurance tests characterize the contract.
