package pipeline

import (
	"context"
	"errors"
	"runtime"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func records(values ...Record) <-chan Record {
	input := make(chan Record, len(values))
	for _, value := range values {
		input <- value
	}
	close(input)
	return input
}

func collect(results <-chan Result) []Result {
	var collected []Result
	for result := range results {
		collected = append(collected, result)
	}
	return collected
}

func TestRejectsInvalidConfiguration(t *testing.T) {
	transform := func(context.Context, Record) (int, error) { return 0, nil }
	if output, err := Process(context.Background(), 0, records(), transform); err == nil || output != nil {
		t.Fatalf("workers=0: output=%v err=%v", output, err)
	}
	if output, err := Process(context.Background(), 1, records(), nil); err == nil || output != nil {
		t.Fatalf("nil transform: output=%v err=%v", output, err)
	}
}

func TestUsesBoundedConcurrencyAndPreservesArrivalOrder(t *testing.T) {
	var active atomic.Int32
	var peak atomic.Int32
	transform := func(_ context.Context, record Record) (int, error) {
		current := active.Add(1)
		for {
			previous := peak.Load()
			if current <= previous || peak.CompareAndSwap(previous, current) {
				break
			}
		}
		defer active.Add(-1)
		// The first record finishes last, making completion-order emission observable.
		time.Sleep(time.Duration(6-record.Value) * 8 * time.Millisecond)
		return record.Value * 10, nil
	}
	input := records(
		Record{Seq: 90, Value: 1},
		Record{Seq: 12, Value: 2},
		Record{Seq: 77, Value: 3},
		Record{Seq: 41, Value: 4},
	)
	output, err := Process(context.Background(), 3, input, transform)
	if err != nil {
		t.Fatal(err)
	}
	got := collect(output)
	wantSeq := []int{90, 12, 77, 41}
	for index, result := range got {
		if result.Seq != wantSeq[index] || result.Value != (index+1)*10 || result.Err != nil {
			t.Fatalf("result[%d]=%+v, want seq=%d value=%d", index, result, wantSeq[index], (index+1)*10)
		}
	}
	if value := peak.Load(); value < 2 || value > 3 {
		t.Fatalf("peak concurrency=%d, want 2..3", value)
	}
}

func TestErrorsAndPanicsStayAttachedAndPipelineContinues(t *testing.T) {
	transform := func(_ context.Context, record Record) (int, error) {
		switch record.Value {
		case 2:
			return 0, errors.New("bad-two")
		case 3:
			panic("boom-three")
		default:
			return record.Value + 100, nil
		}
	}
	output, err := Process(context.Background(), 4, records(
		Record{Seq: 1, Value: 1},
		Record{Seq: 2, Value: 2},
		Record{Seq: 3, Value: 3},
		Record{Seq: 4, Value: 4},
	), transform)
	if err != nil {
		t.Fatal(err)
	}
	got := collect(output)
	if len(got) != 4 || got[0].Value != 101 || got[3].Value != 104 {
		t.Fatalf("pipeline did not continue in order: %+v", got)
	}
	if got[1].Err == nil || got[1].Err.Error() != "bad-two" {
		t.Fatalf("transform error lost: %+v", got[1])
	}
	if got[2].Err == nil || !strings.Contains(got[2].Err.Error(), "boom-three") {
		t.Fatalf("panic not converted: %+v", got[2])
	}
}

func TestCancellationUnblocksSlowConsumerAndCloses(t *testing.T) {
	baseline := runtime.NumGoroutine()
	ctx, cancel := context.WithCancel(context.Background())
	input := make(chan Record, 100)
	for index := range 100 {
		input <- Record{Seq: index + 1000, Value: index}
	}
	close(input)
	transform := func(ctx context.Context, record Record) (int, error) {
		select {
		case <-time.After(3 * time.Millisecond):
			return record.Value, nil
		case <-ctx.Done():
			return 0, ctx.Err()
		}
	}
	output, err := Process(ctx, 5, input, transform)
	if err != nil {
		t.Fatal(err)
	}
	// Do not consume output: force every stage to experience backpressure.
	time.Sleep(20 * time.Millisecond)
	cancel()
	deadline := time.After(500 * time.Millisecond)
	for {
		select {
		case _, open := <-output:
			if !open {
				time.Sleep(20 * time.Millisecond)
				if extra := runtime.NumGoroutine() - baseline; extra > 3 {
					t.Fatalf("possible goroutine leak: baseline=%d now=%d", baseline, runtime.NumGoroutine())
				}
				return
			}
		case <-deadline:
			t.Fatal("output did not close promptly after cancellation")
		}
	}
}
