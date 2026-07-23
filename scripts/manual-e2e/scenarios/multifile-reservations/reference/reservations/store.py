from __future__ import annotations

import asyncio

from .models import Reservation, StorageError


class InMemoryStore:
    def __init__(self, inventory: dict[str, int]) -> None:
        self.inventory = dict(inventory)
        self.reservations: dict[str, Reservation] = {}
        self.lock = asyncio.Lock()
        self.fail_next_save = False
        self._sequence = 0

    def next_sequence(self) -> int:
        self._sequence += 1
        return self._sequence

    async def save_reservation(self, reservation: Reservation) -> None:
        # Yield deliberately so races are observable in tests.
        await asyncio.sleep(0)
        if self.fail_next_save:
            self.fail_next_save = False
            raise StorageError("injected reservation write failure")
        self.reservations[reservation.request_id] = reservation

    async def save_cancellation(self, reservation: Reservation) -> None:
        await asyncio.sleep(0)
        self.reservations[reservation.request_id] = reservation
