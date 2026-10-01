"""Public progress and cancellation contracts; no network or repository fixtures."""

import concurrent.futures
import unittest

import gix


class RuntimeTests(unittest.TestCase):
    def test_progress_snapshots_are_independent(self):
        progress = gix.Progress()
        first = progress.snapshot()
        self.assertEqual(progress.state, "idle")
        self.assertEqual(first, {"state": "idle", "tasks": (), "messages": ()})
        first["state"] = "changed by caller"
        self.assertEqual(progress.snapshot()["state"], "idle")

    def test_token_can_be_cancelled_from_another_thread(self):
        token = gix.CancellationToken()
        self.assertFalse(token.cancelled)
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
            executor.submit(token.cancel).result(timeout=5)
        self.assertTrue(token.cancelled)
        token.cancel()
        self.assertTrue(token.cancelled)

    def test_progress_can_be_polled_from_multiple_threads(self):
        progress = gix.Progress()
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as executor:
            snapshots = list(executor.map(lambda _: progress.snapshot(), range(32)))
        self.assertTrue(all(snapshot["state"] == "idle" for snapshot in snapshots))


if __name__ == "__main__":
    unittest.main()
