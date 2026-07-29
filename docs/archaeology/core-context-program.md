# Code archaeology: Core context program

The prior context block grew around behavior that must remain co-located with
Session state rather than split into speculative cross-crate traits. Its key
history includes long-session compaction/resume hardening (`526fc672`),
contextual routing (`d084b813`), cache-aware affinity (`9a40bbb0`), and
provider recovery. Those changes establish strict invariants:

- request construction is model-specific and bounded after the stable preamble;
- all route inspection/execution inputs come from one readiness snapshot;
- compaction never drops audit history, and undo reloads the durable transcript;
- a context overflow trims/retries before it is treated as provider failure;
- optional memory/recap/suggestion/diagnosis calls are best effort and cannot
  turn a successful main turn into a failure;
- candidate failover and health classifications retain their model/provider
  scopes.

The three-module extraction keeps each policy family deep and private while
retaining `Session` as the state-owning orchestrator. Characterization is the
Core library and long-session endurance suites, including compaction, overflow,
affinity, failover, and post-turn behavior.
