from pathlib import Path
import tempfile
import unittest

import gix


@unittest.skipUnless("dirwalk" in gix.build_features(), "dirwalk feature disabled")
class DirwalkTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name)
        self.repo = gix.init(self.path, options=gix.OpenOptions.isolated())

    def test_walk_outcome_and_ignore_classification(self):
        (self.path / ".gitignore").write_bytes(b"*.log\n")
        (self.path / "untracked").write_bytes(b"content")
        (self.path / "ignored.log").write_bytes(b"content")
        index = self.repo.index_or_empty()
        options = self.repo.dirwalk_options().emit_ignored("matching").emit_untracked("matching")
        progress = gix.Progress()
        walk = self.repo.dirwalk_iter(index, [], options, progress=progress)
        entries = {item.entry.rela_path: item.entry for item in walk}
        self.assertEqual(entries[b"untracked"].status, "Untracked")
        self.assertIn("Ignored", entries[b"ignored.log"].status)
        outcome = walk.into_outcome()
        self.assertIsNotNone(outcome)
        self.assertEqual(len(outcome.index), 0)
        self.assertEqual(outcome.traversal_root, self.path)
        self.assertGreaterEqual(outcome.dirwalk.read_dir_calls, 1)
        self.assertTrue(outcome.pathspec.is_included("untracked", False))
        self.assertTrue(outcome.excludes.at_entry("ignored.log").is_excluded())

    def test_pathspec_and_early_close(self):
        (self.path / "keep").write_bytes(b"yes")
        (self.path / "skip").write_bytes(b"no")
        index = self.repo.index_or_empty()
        with self.repo.dirwalk_iter(index, ["keep"], self.repo.dirwalk_options()) as walk:
            self.assertEqual([item.entry.rela_path for item in walk], [b"keep"])
        walk = self.repo.dirwalk_iter(index, [], self.repo.dirwalk_options())
        walk.close()
        self.assertEqual(list(walk), [])
        with self.assertRaises(ValueError):
            self.repo.dirwalk_options().emit_ignored("invalid")


if __name__ == "__main__":
    unittest.main()
