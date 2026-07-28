#!/usr/bin/env python3
"""Hidden high-contention checks for the long-session reservation scenario."""

from __future__ import annotations

import asyncio
import json
import sys
from pathlib import Path


async def verify(workspace: Path) -> dict[str, int | bool]:
    sys.path.insert(0, str(workspace))
    from reservations import (
        Conflict,
        InMemoryStore,
        OutOfStock,
        ReservationService,
        StorageError,
    )

    rare_store = InMemoryStore({"rare": 1})
    rare_service = ReservationService(rare_store)
    contention = await asyncio.gather(
        *(
            rare_service.reserve(f"rare-{index}", "rare", 1)
            for index in range(100)
        ),
        return_exceptions=True,
    )
    successes = [result for result in contention if not isinstance(result, Exception)]
    out_of_stock = [result for result in contention if isinstance(result, OutOfStock)]
    assert len(successes) == 1, f"expected one contention winner, got {len(successes)}"
    assert len(out_of_stock) == 99, f"expected 99 OutOfStock results, got {len(out_of_stock)}"
    assert rare_store.inventory["rare"] == 0
    assert len(rare_store.reservations) == 1

    store = InMemoryStore({"widget": 10})
    service = ReservationService(store)
    duplicates = await asyncio.gather(
        *(service.reserve("same-request", "widget", 4) for _ in range(100))
    )
    assert all(result == duplicates[0] for result in duplicates)
    assert store.inventory["widget"] == 6
    assert len(store.reservations) == 1

    for sku, quantity in (("widget", 3), ("other", 4)):
        try:
            await service.reserve("same-request", sku, quantity)
        except Conflict:
            pass
        else:
            raise AssertionError(f"conflicting duplicate unexpectedly succeeded: {sku}/{quantity}")

    cancellations = await asyncio.gather(
        *(service.cancel("same-request") for _ in range(100))
    )
    assert all(not reservation.active for reservation in cancellations)
    assert store.inventory["widget"] == 10

    class InterleavingStore(InMemoryStore):
        def __init__(self) -> None:
            super().__init__({"widget": 100})
            self.persistence_order: list[str] = []

        async def save_reservation(self, reservation):
            self.persistence_order.append("reserve")
            await super().save_reservation(reservation)

        async def save_cancellation(self, reservation):
            self.persistence_order.append("cancel")
            await super().save_cancellation(reservation)

    interleaved_store = InterleavingStore()
    interleaved_service = ReservationService(interleaved_store)
    await asyncio.gather(
        *(
            interleaved_service.reserve(f"existing-{index}", "widget", 1)
            for index in range(50)
        )
    )
    interleaved_store.persistence_order.clear()

    # Hold the service's shared lock while queueing calls in an exact alternating order. Releasing
    # it makes 100 already-overlapping operations enter their critical sections cancel/reserve,
    # cancel/reserve, ... rather than relying on scheduler luck that could group every reserve
    # before every cancellation.
    await interleaved_store.lock.acquire()
    interleaved_calls: list[asyncio.Task] = []
    try:
        for index in range(50):
            interleaved_calls.append(
                asyncio.create_task(interleaved_service.cancel(f"existing-{index}"))
            )
            await asyncio.sleep(0)
            interleaved_calls.append(
                asyncio.create_task(
                    interleaved_service.reserve(f"replacement-{index}", "widget", 1)
                )
            )
            await asyncio.sleep(0)
    finally:
        interleaved_store.lock.release()
    await asyncio.gather(*interleaved_calls)

    expected_order = ["cancel", "reserve"] * 50
    assert interleaved_store.persistence_order == expected_order
    assert interleaved_store.inventory["widget"] == 50
    assert len(interleaved_store.reservations) == 100
    assert all(
        not interleaved_store.reservations[f"existing-{index}"].active
        and interleaved_store.reservations[f"replacement-{index}"].active
        for index in range(50)
    )

    failure_store = InMemoryStore({"widget": 2})
    failure_service = ReservationService(failure_store)
    failure_store.fail_next_save = True
    try:
        await failure_service.reserve("fails", "widget", 2)
    except StorageError:
        pass
    else:
        raise AssertionError("injected storage failure unexpectedly succeeded")
    assert failure_store.inventory == {"widget": 2}
    assert failure_store.reservations == {}

    cancellation_failure_store = InMemoryStore({"widget": 2})
    cancellation_failure_service = ReservationService(cancellation_failure_store)
    await cancellation_failure_service.reserve("cancel-fails", "widget", 1)
    cancellation_failure_store.fail_next_save = True
    try:
        await cancellation_failure_service.cancel("cancel-fails")
    except StorageError:
        pass
    else:
        raise AssertionError("injected cancellation write failure unexpectedly succeeded")
    assert cancellation_failure_store.inventory == {"widget": 1}
    assert cancellation_failure_store.reservations["cancel-fails"].active

    return {
        "contention_requests": len(contention),
        "contention_winners": len(successes),
        "duplicate_requests": len(duplicates),
        "concurrent_cancellations": len(cancellations),
        "interleaved_reserve_cancels": len(interleaved_calls),
        "interleaving_verified": True,
        "rollback_verified": True,
        "cancellation_rollback_verified": True,
    }


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} <workspace>")
    result = asyncio.run(verify(Path(sys.argv[1]).resolve()))
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
