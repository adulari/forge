# Ordered pipeline reference

The fixture emits completion order instead of input order, leaks an internal index as `Seq`, and has
unsafe cancellation/backpressure behavior. The live run encountered and recovered from a 120-second
test deadlock. The saved reference passes:

```bash
cd reference
gofmt -d pipeline/pipeline.go
go vet ./...
go test -race ./...
```

## Verified unpinned-mesh result (2026-07-23)

A fresh real-TUI run repaired worker lifecycle, input-order buffering, public sequence attribution,
panic/error propagation, cancellation, and bounded backpressure in 107 seconds. Independent runner
acceptance passed formatting, `go vet ./...`, and `go test -race ./...`. The automatic persistence
audit found all 19 parent tool envelopes/executions structurally valid with zero non-OK outcomes.
The full workspace, resumable session, raw TUI stream, and progress timeline are retained as
`go-ordered-pipeline-20260723T033521Z-1894454` in Forge's persistent `manual-e2e-runs/` directory.
