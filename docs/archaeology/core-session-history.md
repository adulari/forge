# Code archaeology: Core session history

History lifecycle is a single correctness boundary. Checkpoints carry database
sequence numbers, not transcript indexes; after compaction/resume their mapping
requires the live transcript offset. Undo must use the real turn's stored user
sequence rather than a synthetic autofix user message. File restores execute
newest-first so an earlier turn's pre-turn bytes are the final restored state.

Workspace transitions also retain ordering: tools rebind before the public
identity changes, the lattice and watcher must be rebuilt for the new root, and
cached branch/project guidance must be refreshed. Resumes rehydrate persisted
messages/tasks and emit SessionStarted before the task-list re-display. The
extraction leaves those bodies intact and characterizes them via Core rewind,
compact/resume, workspace, and endurance tests.
