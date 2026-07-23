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
