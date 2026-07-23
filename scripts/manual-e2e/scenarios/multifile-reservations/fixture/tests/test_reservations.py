from __future__ import annotations

import asyncio
import unittest

from reservations import (
    Conflict,
    InMemoryStore,
    InvalidRequest,
    NotFound,
    OutOfStock,
    ReservationService,
    StorageError,
)


class ReservationServiceTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self) -> None:
        self.store = InMemoryStore({"widget": 3, "cable": 2})
        self.service = ReservationService(self.store)

    async def test_validates_positive_quantity_and_known_sku(self) -> None:
        for quantity in (0, -1, -20):
            with self.subTest(quantity=quantity), self.assertRaises(InvalidRequest):
                await self.service.reserve(f"bad-{quantity}", "widget", quantity)
        with self.assertRaises(NotFound):
            await self.service.reserve("missing", "unknown", 1)
        self.assertEqual(self.store.inventory, {"widget": 3, "cable": 2})

    async def test_same_request_is_idempotent(self) -> None:
        first = await self.service.reserve("req-1", "widget", 2)
        second = await self.service.reserve("req-1", "widget", 2)
        self.assertIs(first, second)
        self.assertEqual(self.store.inventory["widget"], 1)

    async def test_same_request_with_different_payload_conflicts(self) -> None:
        await self.service.reserve("req-1", "widget", 1)
        for sku, quantity in (("widget", 2), ("cable", 1)):
            with self.subTest(sku=sku, quantity=quantity), self.assertRaises(Conflict):
                await self.service.reserve("req-1", sku, quantity)
        self.assertEqual(self.store.inventory, {"widget": 2, "cable": 2})

    async def test_concurrent_requests_cannot_oversell(self) -> None:
        store = InMemoryStore({"rare": 1})
        service = ReservationService(store)
        results = await asyncio.gather(
            service.reserve("a", "rare", 1),
            service.reserve("b", "rare", 1),
            return_exceptions=True,
        )
        self.assertEqual(sum(not isinstance(item, Exception) for item in results), 1)
        self.assertEqual(sum(isinstance(item, OutOfStock) for item in results), 1)
        self.assertEqual(store.inventory["rare"], 0)
        self.assertEqual(len(store.reservations), 1)

    async def test_concurrent_duplicate_request_decrements_once(self) -> None:
        results = await asyncio.gather(
            self.service.reserve("same", "widget", 2),
            self.service.reserve("same", "widget", 2),
        )
        self.assertEqual(results[0], results[1])
        self.assertEqual(self.store.inventory["widget"], 1)
        self.assertEqual(len(self.store.reservations), 1)

    async def test_storage_failure_rolls_back_inventory(self) -> None:
        self.store.fail_next_save = True
        with self.assertRaises(StorageError):
            await self.service.reserve("req-1", "widget", 2)
        self.assertEqual(self.store.inventory["widget"], 3)
        self.assertNotIn("req-1", self.store.reservations)

    async def test_cancel_is_idempotent_even_concurrently(self) -> None:
        await self.service.reserve("req-1", "widget", 2)
        results = await asyncio.gather(
            self.service.cancel("req-1"),
            self.service.cancel("req-1"),
        )
        self.assertTrue(all(not result.active for result in results))
        self.assertEqual(self.store.inventory["widget"], 3)
        self.assertFalse(self.store.reservations["req-1"].active)

    async def test_cancel_unknown_and_active_ordering(self) -> None:
        with self.assertRaises(NotFound):
            await self.service.cancel("absent")
        first = await self.service.reserve("first", "widget", 1)
        await self.service.reserve("second", "cable", 1)
        third = await self.service.reserve("third", "widget", 1)
        await self.service.cancel("second")
        active = await self.service.active_reservations()
        self.assertEqual(active, [first, third])


if __name__ == "__main__":
    unittest.main()
