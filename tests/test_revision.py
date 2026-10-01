from pathlib import Path
import tempfile
import unittest

import gix


@unittest.skipUnless("revision" in gix.build_features(), "revision disabled")
class RevisionTests(unittest.TestCase):
    def test_native_revision_walk_laziness_and_cancellation(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = gix.init(Path(directory) / "repo", options=gix.OpenOptions.isolated().config_overrides([
                "user.name=Example", "user.email=example@example.com",
            ]))
            tree = repo.empty_tree().id
            first = repo.commit("HEAD", "first", tree, [])
            walk = repo.rev_walk(["HEAD"]).all()
            second = repo.commit("HEAD", "second", tree, [first])
            self.assertEqual(next(walk).id, second)  # resolve on first demand
            self.assertEqual(next(walk).id, first)
            with self.assertRaises(StopIteration):
                next(walk)
            self.assertEqual(repo.rev_parse_single("HEAD~1"), first)
            self.assertEqual(repo.rev_parse("HEAD").single(), second)
            self.assertIsNone(repo.rev_parse("HEAD~1..HEAD").single())
            self.assertEqual(repo.merge_base(first, second), first)
            self.assertEqual(repo.merge_base_octopus([first, second]), first)
            self.assertEqual([x.id for x in repo.find_commit(second).ancestors().with_hidden([first]).all()], [second])
            cancel = gix.CancellationToken()
            progress = gix.Progress()
            walk = repo.rev_walk(["HEAD"]).all(cancel=cancel, progress=progress)
            self.assertEqual(progress.state, "idle")
            self.assertEqual(next(walk).id, second)
            cancel.cancel()
            with self.assertRaises(gix.CancelledError):
                next(walk)
            self.assertEqual(progress.state, "cancelled")
            walk.close()
            self.assertFalse(repo.is_shallow())
            self.assertIsNone(repo.shallow_commits())


if __name__ == "__main__":
    unittest.main()
