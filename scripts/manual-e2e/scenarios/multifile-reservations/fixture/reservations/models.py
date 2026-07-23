from __future__ import annotations

from dataclasses import dataclass, replace


class ReservationError(Exception):
    pass


class InvalidRequest(ReservationError):
    pass


class NotFound(ReservationError):
    pass


class OutOfStock(ReservationError):
    pass


class Conflict(ReservationError):
    pass


class StorageError(ReservationError):
    pass


@dataclass(frozen=True, slots=True)
class Reservation:
    request_id: str
    sku: str
    quantity: int
    sequence: int
    active: bool = True

    def cancelled(self) -> "Reservation":
        return replace(self, active=False)
