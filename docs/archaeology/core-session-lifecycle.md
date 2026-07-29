# Code archaeology: Core Session lifecycle

`Session::start`, `resume`, and the shared constructor establish the durable
identity from which every later turn derives. History shows compaction made the
next database sequence independent from the visible loaded transcript: resume
must ask Store for `MAX(seq)+1`, not use message count. Workspace hardening
requires persisted canonical CWD instead of ambient process CWD; permission mode
must likewise restore from the session row.

The common builder establishes provider/router/tool/presenter references,
cache-stable project metadata, context/contract audit state, and emits exactly
one `SessionStarted` event after the object is complete. It must not be split
into independent partial initializers, which would obscure ordering and reopen
construction windows.

The lifecycle extraction keeps this deep, cohesive owner intact and leaves turn
orchestration in the Session program. Core and endurance coverage exercise
fresh/resumed sessions, compaction/resume sequence correctness, distinct daemon
workspaces, interruption recovery, and long-session isolation.
