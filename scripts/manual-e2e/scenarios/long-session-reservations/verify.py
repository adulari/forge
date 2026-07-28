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

    return {
        "contention_requests": len(contention),
        "contention_winners": len(successes),
        "duplicate_requests": len(duplicates),
        "concurrent_cancellations": len(cancellations),
        "rollback_verified": True,
    }


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} <workspace>")
    result = asyncio.run(verify(Path(sys.argv[1]).resolve()))
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
