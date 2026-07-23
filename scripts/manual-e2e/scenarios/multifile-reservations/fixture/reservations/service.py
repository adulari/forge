from __future__ import annotations

import asyncio

from .models import InvalidRequest, NotFound, OutOfStock, Reservation
from .store import InMemoryStore


class ReservationService:
    def __init__(self, store: InMemoryStore) -> None:
        self.store = store

    async def reserve(self, request_id: str, sku: str, quantity: int) -> Reservation:
        if quantity == 0:
            raise InvalidRequest("quantity cannot be zero")
        if sku not in self.store.inventory:
            raise NotFound(sku)

        existing = self.store.reservations.get(request_id)
        if existing is not None:
            return existing

        available = self.store.inventory[sku]
        await asyncio.sleep(0)
        if available < quantity:
            raise OutOfStock(sku)

        self.store.inventory[sku] = available - quantity
        reservation = Reservation(
            request_id=request_id,
            sku=sku,
            quantity=quantity,
            sequence=self.store.next_sequence(),
        )
        await self.store.save_reservation(reservation)
        return reservation

    async def cancel(self, request_id: str) -> Reservation:
        reservation = self.store.reservations.get(request_id)
        if reservation is None:
            raise NotFound(request_id)
        self.store.inventory[reservation.sku] += reservation.quantity
        cancelled = reservation.cancelled()
        await self.store.save_cancellation(cancelled)
        return cancelled

    async def active_reservations(self) -> list[Reservation]:
        await asyncio.sleep(0)
        return list(reversed(self.store.reservations.values()))
