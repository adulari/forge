from __future__ import annotations

from .models import Conflict, InvalidRequest, NotFound, OutOfStock, Reservation
from .store import InMemoryStore


class ReservationService:
    def __init__(self, store: InMemoryStore) -> None:
        self.store = store

    async def reserve(self, request_id: str, sku: str, quantity: int) -> Reservation:
        async with self.store.lock:
            existing = self.store.reservations.get(request_id)
            if existing is not None:
                if existing.sku != sku or existing.quantity != quantity:
                    raise Conflict(request_id)
                return existing

            if not isinstance(quantity, int) or isinstance(quantity, bool) or quantity <= 0:
                raise InvalidRequest("quantity must be positive")
            if sku not in self.store.inventory:
                raise NotFound(sku)

            available = self.store.inventory[sku]
            if available < quantity:
                raise OutOfStock(sku)

            sequence = self.store.next_sequence()
            reservation = Reservation(
                request_id=request_id,
                sku=sku,
                quantity=quantity,
                sequence=sequence,
            )
            await self.store.save_reservation(reservation)
            self.store.inventory[sku] = available - quantity
            return reservation

    async def cancel(self, request_id: str) -> Reservation:
        async with self.store.lock:
            reservation = self.store.reservations.get(request_id)
            if reservation is None:
                raise NotFound(request_id)
            if not reservation.active:
                return reservation

            cancelled = reservation.cancelled()
            await self.store.save_cancellation(cancelled)
            self.store.inventory[reservation.sku] += reservation.quantity
            return cancelled

    async def active_reservations(self) -> list[Reservation]:
        async with self.store.lock:
            return [
                reservation
                for reservation in self.store.reservations.values()
                if reservation.active
            ]
