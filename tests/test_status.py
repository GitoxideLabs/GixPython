from pathlib import Path
import tempfile
import unittest

import gix


@unittest.skipUnless("status" in gix.build_features(), "status feature disabled")
class StatusTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name)
        self.repo = gix.init(self.path, options=gix.OpenOptions.isolated())
        signature = gix.Signature("Test", "test@example.com", 1700000000)
        self.blob = self.repo.write_blob(b"before\n")
        with self.repo.empty_tree().edit() as editor:
            editor.upsert("tracked", "blob", self.blob)
            self.tree = self.repo.find_tree(editor.write())
        self.repo.commit_as(signature, signature, "HEAD", "initial", self.tree)
        self.repo.index_from_tree(self.tree).write()
        (self.path / "tracked").write_bytes(b"before\n")

    def test_clean_status_outcome_can_refresh_index_metadata(self):
        progress = gix.Progress()
        cursor = self.repo.status(progress).into_iter()
        self.assertEqual(progress.state, "idle")
        self.assertIsNone(cursor.outcome_mut())
        self.assertEqual(list(cursor), [])
        self.assertEqual(progress.state, "succeeded")
        outcome = cursor.outcome_mut()
        self.assertIsNotNone(outcome)
        self.assertEqual(len(outcome.worktree_index), 1)
        self.assertTrue(outcome.has_changes())
        self.assertTrue(outcome.write_changes())
        self.assertFalse(outcome.has_changes())
        self.assertFalse(outcome.write_changes())
        self.assertFalse(self.repo.is_dirty())
        self.assertIsNotNone(cursor.into_outcome())
        self.assertIsNone(cursor.outcome_mut())

    def test_staged_and_worktree_changes_retain_details(self):
        staged = self.repo.write_blob(b"staged\n")
        index = self.repo.open_index()
        index.entry_mut_by_path_and_stage("tracked", 0).id = staged
        index.write()
        (self.path / "tracked").write_bytes(b"working\n")
        (self.path / "untracked").write_bytes(b"new\n")
        items = list(self.repo.status().untracked_files("files").into_iter())
        staged_item = next(item for item in items if item.kind == "TreeIndex")
        self.assertEqual(staged_item.location(), b"tracked")
        self.assertEqual(staged_item.details["previous_id"], self.blob)
        self.assertEqual(staged_item.details["id"], staged)
        self.assertEqual(staged_item.summary(), "Modified")
        worktree = next(item for item in items if item.kind == "IndexWorktree" and item.location() == b"tracked")
        self.assertEqual(worktree.summary(), "Modified")
        self.assertEqual(worktree.details["entry"].id, staged)
        self.assertTrue(worktree.details["status"]["content_changed"])
        self.assertTrue(any(item.location() == b"untracked" and item.summary() == "Added" for item in items))
        self.assertTrue(self.repo.is_dirty())
        tracked_only = list(self.repo.status().untracked_files("none").into_index_worktree_iter())
        self.assertEqual([(item.kind, item.location()) for item in tracked_only], [("IndexWorktree", b"tracked")])

    def test_pathspec_ignored_and_precancelled_status(self):
        (self.path / ".gitignore").write_text("ignored\n")
        (self.path / "ignored").write_text("hidden")
        default = list(self.repo.status().into_iter(["ignored"]))
        self.assertEqual(default, [])
        ignored = list(self.repo.status().dirwalk_options(emit_ignored=True).into_iter(["ignored"]))
        self.assertEqual(len(ignored), 1)
        self.assertIsNone(ignored[0].summary())
        self.assertTrue(ignored[0].details["entry"]["status"].startswith("Ignored"))
        progress = gix.Progress()
        token = gix.CancellationToken()
        token.cancel()
        cursor = self.repo.status(progress, cancel=token).into_iter()
        with self.assertRaises(gix.CancelledError):
            next(cursor)
        self.assertEqual(progress.state, "idle")
        self.assertIsNone(cursor.into_outcome())


if __name__ == "__main__":
    unittest.main()
