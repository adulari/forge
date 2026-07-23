# Reservations reference

The fixture begins with ten failures involving async overselling, invalid quantity handling,
conflicting request IDs, rollback, cancellation idempotency, and stable ordering. The saved reference
passes all eight test methods:

```bash
cd reference
python3 -m unittest discover -v
```

## Verified unpinned-mesh result (2026-07-23)

A fresh real-TUI run coordinated independent Codex and NVIDIA NIM inspections, then repaired the
service across its model, store, and service modules. The runner independently repeated all 8
async overselling, duplicate/conflicting request, rollback, validation, cancellation, and ordering
tests successfully. The automatic parent-session audit found all 21 persisted tool
envelopes/executions structurally valid with zero non-OK outcomes; direct database aggregation also
confirmed all 21 child-session tool executions were OK across Codex, NVIDIA Minimax, and NVIDIA
DeepSeek. The full workspace, resumable session, raw TUI stream, and progress timeline are retained
as `multifile-reservations-20260723T033736Z-1898494` in Forge's persistent `manual-e2e-runs/`
directory.
