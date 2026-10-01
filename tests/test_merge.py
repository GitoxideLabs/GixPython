import tempfile
import unittest
import gix


@unittest.skipUnless("merge" in gix.build_features(), "merge disabled")
class MergeTests(unittest.TestCase):
    def test_conflicts_remain_editable_and_options_preserve_information(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = gix.init(directory, options=gix.OpenOptions.isolated().config_overrides([
                "user.name=Example", "user.email=example@example.com",
            ]))
            def tree(data):
                with repo.empty_tree().edit() as editor:
                    editor.upsert(b"file", "blob", repo.write_blob(data))
                    return editor.write()
            base_tree = tree(b"base\n")
            ours_tree = tree(b"ours\n")
            theirs_tree = tree(b"theirs\n")
            base = repo.new_commit("base", base_tree, [])
            ours = repo.new_commit("ours", ours_tree, [base])
            theirs = repo.new_commit("theirs", theirs_tree, [base])
            progress = gix.Progress()
            token = gix.CancellationToken()
            merged = repo.merge_trees(base_tree, ours_tree, theirs_tree, progress=progress, cancel=token,
                labels=gix.MergeLabels(current=b"our branch", other=b"their branch"))
            self.assertEqual(progress.state, "succeeded")
            token.cancel()  # The returned editor has its own lifetime.
            self.assertTrue(merged.has_unresolved_conflicts(gix.TreatAsUnresolved.git()))
            conflict = merged.conflicts[0]
            self.assertEqual(conflict.resolution["kind"], "OursModifiedTheirsModifiedThenBlobContentMerge")
            self.assertEqual(conflict.ours.location(), b"file")
            self.assertIsNotNone(conflict.content_merge())
            with merged.tree as editor:
                unresolved_tree = editor.write()
                index = repo.index_from_tree(unresolved_tree)
                self.assertTrue(merged.index_changed_after_applying_conflicts(index, gix.TreatAsUnresolved.git()))
                self.assertEqual([e.stage() for e in index.entries()], [1, 2, 3])
                editor.upsert("file", "blob", repo.write_blob(b"resolved\n"))
                resolved_tree = editor.write()
            self.assertEqual(progress.state, "succeeded")
            self.assertEqual(repo.find_tree(resolved_tree).find_entry(b"file").object().data, b"resolved\n")
            options = gix.CommitMergeOptions(repo.tree_merge_options().with_file_favor("ours"))
            merged = repo.merge_commits(ours, theirs, options=options)
            self.assertEqual(merged.merge_base_tree_id, base_tree)
            self.assertEqual(merged.merge_bases, [base.id])
            self.assertFalse(merged.tree_merge.has_unresolved_conflicts(gix.TreatAsUnresolved.git()))
            self.assertTrue(merged.tree_merge.has_unresolved_conflicts(gix.TreatAsUnresolved.forced_resolution()))
            with merged.tree_merge.tree as editor:
                self.assertEqual(repo.find_tree(editor.write()).find_entry("file").object().data, b"ours\n")
            virtual = repo.virtual_merge_base([base])
            self.assertEqual(virtual.commit_id, base.id)
            self.assertEqual(virtual.tree_id, base_tree)


if __name__ == "__main__":
    unittest.main()
