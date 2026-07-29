# Code archaeology: Core delegated-work orchestration

Subagents and workflows are not generic provider calls: each child has a
persisted parent link, its own Mesh route and Store session, a bounded depth,
workspace/tool confinement, and ordered presenter updates. Parent cancellation
must terminate in-flight workers and mark durable workflow work interrupted;
child follow-up resolves stable identity before carrying context forward.

Duel execution additionally maintains worktree/candidate lifetime until the
parent surface resolves it. These coupled resource and event-order guarantees
justify a private deep Session orchestration owner. Core subagent/workflow/duel
coverage and long-session endurance characterize the extraction.
