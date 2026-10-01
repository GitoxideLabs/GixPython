from pathlib import Path
import tempfile
import unittest

import gix


@unittest.skipUnless("blob-diff" in gix.build_features(), "blob-diff feature disabled")
class DiffTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.repo = gix.init_bare(Path(self.directory.name) / "repo", options=gix.OpenOptions.isolated())
        self.old_blob = self.repo.write_blob(b"one\ntwo\n")
        self.new_blob = self.repo.write_blob(b"one\nthree\n")
        with self.repo.empty_tree().edit() as editor:
            editor.upsert("file", "blob", self.old_blob)
            self.old_tree = self.repo.find_tree(editor.write())
            editor.upsert("file", "blob", self.new_blob)
            self.new_tree = self.repo.find_tree(editor.write())

    def test_tree_changes_and_native_line_diff(self):
        changes = self.repo.diff_tree_to_tree(self.old_tree, self.new_tree)
        self.assertEqual(len(changes), 1)
        change = changes[0]
        self.assertEqual((change.kind, change.location(), change.id()), ("Modification", b"file", self.new_blob))
        self.assertEqual(change.details["previous_id"], self.old_blob)
        cache = self.repo.diff_resource_cache_for_tree_diff()
        blob_diff = change.diff(cache)
        counts = blob_diff.line_counts()
        self.assertEqual((counts.removals, counts.insertions, counts.before, counts.after), (1, 1, 2, 2))
        with blob_diff.lines() as hunks:
            hunk = next(hunks)
            self.assertEqual((hunk.before, hunk.after), ((1, 2), (1, 2)))
            self.assertEqual(hunk.lines_before, (b"two",))
            self.assertEqual(hunk.lines_after, (b"three",))
            self.assertEqual(list(hunks), [])
        self.assertEqual(cache.resource("old").data, b"one\ntwo\n")
        self.assertEqual(cache.prepare_diff().operation, "InternalDiff")
        stats = self.old_tree.changes().stats(self.new_tree)
        self.assertEqual((stats.files_changed, stats.lines_added, stats.lines_removed), (1, 1, 1))
        cache.clear_resource_cache()

    def test_tree_callback_adapter_is_lazy_and_cancelable(self):
        progress = gix.Progress()
        cursor = self.old_tree.changes().for_each_to_obtain_tree(self.new_tree, progress=progress)
        self.assertEqual(progress.state, "idle")
        self.assertEqual(next(cursor).kind, "Modification")
        self.assertEqual(progress.state, "running")
        cursor.close()
        self.assertEqual(progress.state, "cancelled")
        self.assertEqual(list(cursor), [])
        cache = self.repo.diff_resource_cache_for_tree_diff()
        changes = self.old_tree.changes().for_each_to_obtain_tree_with_cache(self.new_tree, cache)
        self.assertEqual([change.location() for change in changes], [b"file"])

    def test_rename_selection_and_binary_resources(self):
        with self.old_tree.edit() as editor:
            editor.remove("file").upsert("renamed", "blob", self.old_blob)
            renamed = self.repo.find_tree(editor.write())
        no_renames = self.repo.diff_tree_to_tree(self.old_tree, renamed, gix.DiffOptions())
        self.assertEqual({change.kind for change in no_renames}, {"Deletion", "Addition"})
        renames = self.repo.diff_tree_to_tree(self.old_tree, renamed, gix.DiffOptions().track_rewrites(gix.Rewrites()))
        self.assertEqual(len(renames), 1)
        self.assertEqual(renames[0].kind, "Rewrite")
        self.assertEqual(renames[0].source_location(), b"file")
        cache = self.repo.diff_resource_cache()
        binary = self.repo.write_blob(b"\x00binary")
        cache.set_resource(binary, 0o100644, "binary", "old")
        cache.set_resource(self.new_blob, 0o100644, "binary", "new")
        prepared = cache.prepare_diff()
        self.assertEqual(prepared.operation, "SourceOrDestinationIsBinary")
        self.assertEqual(prepared.old.kind, "Binary")
        self.assertEqual(prepared.old.binary_size, 7)
        self.assertIsNone(prepared.old.data)


if __name__ == "__main__":
    unittest.main()
