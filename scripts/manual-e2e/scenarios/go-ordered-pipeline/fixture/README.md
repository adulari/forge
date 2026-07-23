# Ordered Concurrent Pipeline

Implement `pipeline.Process` as a production-quality bounded worker pipeline.

Contract:

- Reject `workers < 1` and a nil transform without starting goroutines.
- Execute at most `workers` transforms concurrently, while actually using concurrency when work is
  available.
- Emit one result per accepted input record in **input arrival order**, even when later transforms
  finish first. `Result.Seq` must preserve the source record's sequence value.
- Preserve transform errors on their corresponding result and continue processing later records.
- Convert a transform panic into an error result for that record; do not crash or abandon the
  remaining stream.
- Respect context cancellation while feeding, working, reordering, and sending to a slow consumer.
  Close the output channel promptly and do not leak goroutines.
- Never close or mutate the caller-owned input channel.

Run `go test -race ./...` before finishing.
