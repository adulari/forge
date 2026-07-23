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
	seq   int
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
	completedResults := make(chan completed)
	output := make(chan Result)

	var workerGroup sync.WaitGroup
	workerGroup.Add(workers)
	for range workers {
		go func() {
			defer workerGroup.Done()
			for {
				select {
				case <-ctx.Done():
					return
				case item, open := <-jobs:
					if !open {
						return
					}
					value, err := runTransform(ctx, transform, item)
					result := completed{index: item.index, seq: item.record.Seq, value: value, err: err}
					select {
					case completedResults <- result:
					case <-ctx.Done():
						return
					}
				}
			}
		}()
	}

	go func() {
		defer close(jobs)

		index := 0
		for {
			select {
			case <-ctx.Done():
				return
			default:
			}

			select {
			case <-ctx.Done():
				return
			case record, open := <-input:
				if !open {
					return
				}
				item := job{index: index, record: record}
				index++
				select {
				case jobs <- item:
				case <-ctx.Done():
					return
				}
			}
		}
	}()

	go func() {
		workerGroup.Wait()
		close(completedResults)
	}()

	go func() {
		defer close(output)

		next := 0
		pending := make(map[int]completed)
		for {
			select {
			case <-ctx.Done():
				return
			case item, open := <-completedResults:
				if !open {
					return
				}
				pending[item.index] = item
				for {
					ready, exists := pending[next]
					if !exists {
						break
					}
					delete(pending, next)
					next++
					result := Result{Seq: ready.seq, Value: ready.value, Err: ready.err}
					select {
					case output <- result:
					case <-ctx.Done():
						return
					}
				}
			}
		}
	}()

	return output, nil
}
