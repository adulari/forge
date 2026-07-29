# Core text policy extraction

This phase moves deterministic recap, completion-recap, suggestion sanitizer,
and project-memory scope policy into private `text_policy.rs`. The policies
remain invoked by the same Session program paths.

The owner prevents a side-call recap from changing task-completion truth,
keeps ghost suggestions bounded/non-repetitive, and keys memories to explicit
workspace roots. Core root implementation lines reduce from 10,424 to 10,332;
the extracted owner is 102 lines. Core Clippy, long-session endurance,
formatting, and the architecture guard passed. No prompt text, provider call,
persistence, event ordering, or permission behavior changed.
