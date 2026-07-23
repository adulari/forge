package pipeline

import (
	"context"
	"errors"
	"fmt"
	"sync"
)

type Record struct {
	Seq   int
	Value int
}

type Result struct {
	Seq   int
	Value int
	Err   error
}

type Transform func(context.Context, Record) (int, error)

type job struct {
	index  int
	record Record
}

type completed struct {
	index int
	value int
	err   error
}

func runTransform(ctx context.Context, transform Transform, item job) (value int, err error) {
	defer func() {
		if recovered := recover(); recovered != nil {
			err = fmt.Errorf("transform panic: %v", recovered)
		}
	}()
	return transform(ctx, item.record)
}

// Process is a first draft. It is known not to satisfy the full ordering and lifecycle contract.
func Process(
	ctx context.Context,
	workers int,
	input <-chan Record,
	transform Transform,
) (<-chan Result, error) {
	if workers < 1 {
		return nil, errors.New("workers must be positive")
	}
	if transform == nil {
		return nil, errors.New("transform must not be nil")
	}

	jobs := make(chan job)
	done := make(chan completed)
	output := make(chan Result)

	var group sync.WaitGroup
	group.Add(workers)
	for range workers {
		go func() {
			defer group.Done()
			for item := range jobs {
				value, err := runTransform(ctx, transform, item)
				done <- completed{index: item.index, value: value, err: err}
			}
		}()
	}

	go func() {
		index := 0
		for record := range input {
			jobs <- job{index: index, record: record}
			index++
		}
		close(jobs)
		group.Wait()
		close(done)
	}()

	go func() {
		defer close(output)
		for item := range done {
			// BUG: completion order and the internal index leak into the public result.
			output <- Result{Seq: item.index, Value: item.value, Err: item.err}
		}
	}()

	return output, nil
}
