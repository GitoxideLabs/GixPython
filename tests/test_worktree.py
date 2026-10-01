"""Worktree safety and incremental native worktree streams."""

import gc
from pathlib import Path
import tempfile
import tarfile
import unittest
import zipfile

import gix


def fixture(path):
    repo = gix.init(path, options=gix.OpenOptions.isolated().config_overrides([
        "user.name=Worktree Tester", "user.email=worktree@example.invalid", "init.defaultBranch=main"
    ]))
    first = repo.write_blob(b"a" * 200000)
    second = repo.write_blob(b"second\x00\xff")
    tree = repo.write_object("tree", b"100644 first\0" + bytes(first) + b"100644 second\0" + bytes(second))
    signature = gix.Signature("Worktree Tester", "worktree@example.invalid", 1700000000)
    commit = repo.commit_as(signature, signature, "HEAD", "root", tree)
    return repo, tree, commit


class WorktreeTests(unittest.TestCase):
    def test_main_and_bare_worktrees(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo, _, _ = fixture(root / "main")
            worktree = repo.worktree()
            self.assertTrue(worktree.is_main())
            self.assertFalse(worktree.is_locked())
            self.assertIsNone(worktree.id())
            self.assertTrue(worktree.dot_git_exists())
            self.assertEqual(Path(worktree.base()), root / "main")
            self.assertEqual(repo.worktrees(), [])
            self.assertIsNone(repo.worktree_proxy_by_id("missing"))
            self.assertEqual([r.git_dir() for r in repo.worktrees_including_main()], [repo.git_dir()])
            bare = gix.init_bare(root / "bare", options=gix.OpenOptions.isolated())
            self.assertIsNone(bare.worktree())
            self.assertTrue(next(bare.worktrees_including_main()).is_bare())

    @unittest.skipUnless("attributes" in gix.build_features(), "attributes disabled")
    def test_worktree_attribute_and_pathspec_shortcuts_use_native_defaults(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "main"
            repo, tree, _ = fixture(path)
            repo.index_from_tree(tree).write()
            (path / ".gitattributes").write_bytes(b"first text\n")
            (path / ".gitignore").write_bytes(b"*.log\n")
            worktree = repo.worktree()
            stack = worktree.attributes()
            self.assertTrue(stack.at_entry("out.log").is_excluded())
            output = stack.selected_attribute_matches(["text"])
            self.assertTrue(stack.at_entry("first").matching_attributes(output))
            self.assertEqual(next(output.iter_selected()).state, "set")
            override = worktree.attributes(gix.IgnoreSearch.from_overrides(["first"]))
            self.assertTrue(override.at_entry("first").is_excluded())
            attributes = worktree.attributes_only()
            with self.assertRaises(ValueError):
                attributes.at_entry("out.log").is_excluded()
            pathspec = worktree.pathspec(["first"])
            self.assertEqual([pattern.path() for pattern in pathspec.search().patterns()], [b"first"])

    @unittest.skipUnless("worktree-mutation" in gix.build_features(), "worktree mutation disabled")
    def test_add_checkout_branch_occupancy_and_remove_safety(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo, _, commit = fixture(root / "main")
            repo.reference("refs/heads/topic", commit, gix.PreviousValue.Any, "branch")
            linked, outcome = repo.add_worktree(root / "linked", gix.WorktreeHead.Attached("refs/heads/topic"))
            self.assertEqual(outcome.files_updated, 2)
            self.assertEqual(outcome.bytes_written, 200008)
            self.assertEqual((outcome.collisions, outcome.errors), ([], []))
            self.assertEqual((root / "linked" / "second").read_bytes(), b"second\x00\xff")
            self.assertEqual(linked.head_name(), b"refs/heads/topic")
            self.assertFalse(linked.worktree().is_main())
            proxy = repo.worktrees()[0]
            self.assertEqual(proxy.id(), linked.worktree().id())
            self.assertEqual(proxy.into_repo().head_id(), commit)
            self.assertEqual(proxy.into_repo_with_possibly_inaccessible_worktree().head_id(), commit)
            self.assertEqual(Path(proxy.base()).resolve(), (root / "linked").resolve())
            self.assertFalse(proxy.is_prunable())
            self.assertEqual(Path(linked.main_repo().git_dir()).resolve(), Path(repo.git_dir()).resolve())
            self.assertEqual(len(list(linked.worktrees_including_main())), 2)
            with self.assertRaises(gix.Error):
                repo.add_worktree(root / "duplicate", gix.WorktreeHead.Attached("refs/heads/topic"))
            with self.assertRaises(gix.Error):
                repo.delete_local_branches(["refs/heads/topic"])
            (root / "linked" / "untracked").write_text("keep")
            with self.assertRaises(gix.Error):
                repo.remove_worktree(root / "linked")
            self.assertTrue((root / "linked" / "untracked").exists())
            (Path(proxy.git_dir()) / "locked").write_bytes(b"keep this worktree")
            self.assertTrue(proxy.is_locked())
            self.assertEqual(proxy.lock_reason(), b"keep this worktree")
            with self.assertRaises(gix.Error):
                repo.remove_worktree(root / "linked", gix.WorktreeRemoveForce.DiscardChanges)
            repo.remove_worktree(root / "linked", gix.WorktreeRemoveForce.OverrideLock)
            self.assertFalse((root / "linked").exists())
            self.assertEqual(repo.worktrees(), [])
            self.assertEqual(repo.find_reference("topic").id(), commit)

    @unittest.skipUnless("worktree-mutation" in gix.build_features(), "worktree mutation disabled")
    def test_cancelled_checkout_keeps_destination_absent(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo, _, commit = fixture(root / "main")
            token = gix.CancellationToken()
            token.cancel()
            with self.assertRaises(gix.CancelledError):
                repo.add_worktree(root / "cancelled", gix.WorktreeHead.Detached(commit), cancel=token)
            self.assertFalse((root / "cancelled").exists())
            self.assertEqual(repo.worktrees(), [])

    @unittest.skipUnless("worktree-mutation" in gix.build_features(), "worktree mutation disabled")
    def test_prepared_remove_captures_target_and_preserves_native_safety(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo, _, commit = fixture(root / "main")
            linked, _ = repo.add_worktree(root / "linked", gix.WorktreeHead.Detached(commit))
            progress = gix.Progress()
            with repo.prepare_remove_worktree(root / "linked", progress=progress) as target:
                self.assertEqual(Path(target.base()).resolve(), (root / "linked").resolve())
                self.assertEqual(target.repository().head_id(), commit)
                self.assertIs(target.options(gix.WorktreeRemoveOptions(thread_limit=1)), target)
                target.remove()
                self.assertEqual(progress.state, "succeeded")
            self.assertFalse((root / "linked").exists())
            with self.assertRaises(RuntimeError):
                target.repository()
            repo.add_worktree(root / "linked", gix.WorktreeHead.Detached(commit))
            with repo.prepare_remove_worktree(root / "linked") as target:
                (root / "linked" / "later").write_bytes(b"keep")
                with self.assertRaises(gix.Error):
                    target.remove()
            self.assertEqual((root / "linked" / "later").read_bytes(), b"keep")
            repo.remove_worktree(root / "linked", gix.WorktreeRemoveForce.DiscardChanges)


@unittest.skipUnless("worktree-stream" in gix.build_features(), "worktree stream disabled")
class WorktreeStreamTests(unittest.TestCase):
    def test_partial_reads_advance_drains_and_invalidates_previous_reader(self):
        with tempfile.TemporaryDirectory() as directory:
            repo, tree, _ = fixture(Path(directory) / "main")
            progress = gix.Progress()
            stream, index = repo.worktree_stream(tree, progress=progress)
            with stream:
                first = stream.next_entry()
                self.assertEqual(first.relative_path(), b"first")
                self.assertEqual(first.mode, 0o100644)
                self.assertEqual(first.bytes_remaining(), 200000)
                self.assertEqual(first.read(0), b"")
                self.assertEqual(first.bytes_remaining(), 200000)
                self.assertEqual(first.read(13), b"a" * 13)
                second = stream.next_entry()
                with self.assertRaises(ValueError):
                    first.read()
                self.assertEqual(second.relative_path(), b"second")
                self.assertEqual(second.read(3), b"sec")
                self.assertEqual(second.read(), b"ond\x00\xff")
                self.assertEqual(second.read(), b"")
                self.assertIsNone(stream.next_entry())
                self.assertEqual(progress.state, "succeeded")
                self.assertIsNone(stream.next_entry())
                with self.assertRaises(ValueError):
                    second.bytes_remaining()
            with self.assertRaises(ValueError):
                stream.next_entry()

    def test_close_and_drop_invalidate_entries_and_cancellation_is_reported(self):
        with tempfile.TemporaryDirectory() as directory:
            repo, tree, _ = fixture(Path(directory) / "main")
            progress = gix.Progress()
            token = gix.CancellationToken()
            stream, _ = repo.worktree_stream(tree, progress=progress, cancel=token)
            first = stream.next_entry()
            self.assertEqual(first.read(1), b"a")
            stream.close()
            self.assertEqual(progress.state, "cancelled")
            with self.assertRaises(ValueError):
                first.read(1)
            stream, _ = repo.worktree_stream(tree)
            first = stream.next_entry()
            del stream
            gc.collect()
            with self.assertRaises(ValueError):
                first.read()
            stream, _ = repo.worktree_stream(tree, cancel=token)
            token.cancel()
            with self.assertRaises(gix.CancelledError):
                stream.next_entry()
            stream.close()

    def test_additional_entries_are_before_traversal_only(self):
        with tempfile.TemporaryDirectory() as directory:
            repo, tree, _ = fixture(Path(directory) / "main")
            stream, _ = repo.worktree_stream(tree)
            with stream:
                extra = gix.AdditionalEntry(repo.object_hash().null(), 0o100644, b"extra-\xff", gix.StreamSource.Memory(b"extra bytes"))
                stream.add_entry(extra)
                stream.next_entry()
                with self.assertRaises(ValueError):
                    stream.add_entry(extra)
                stream.next_entry()
                entry = stream.next_entry()
                self.assertEqual(entry.relative_path(), b"extra-\xff")
                self.assertEqual(entry.read(), b"extra bytes")
                self.assertIsNone(stream.next_entry())

    @unittest.skipUnless("worktree-archive" in gix.build_features(), "worktree archive disabled")
    def test_archive_formats(self):
        formats = [("archive-tar", "tar"), ("archive-tar-gz", "tar.gz"), ("archive-zip", "zip")]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo, tree, _ = fixture(root / "main")
            for feature, format in formats:
                if feature not in gix.build_features():
                    continue
                with self.subTest(format=format):
                    progress = gix.Progress()
                    stream, _ = repo.worktree_stream(tree, progress=progress)
                    output = root / ("out." + format)
                    repo.worktree_archive(stream, output, gix.ArchiveOptions(format, tree_prefix=b"prefix/", modification_time=1700000000))
                    self.assertEqual(progress.state, "succeeded")
                    if format == "zip":
                        with zipfile.ZipFile(output) as archive:
                            self.assertEqual(archive.read("prefix/second"), b"second\x00\xff")
                    else:
                        with tarfile.open(output) as archive:
                            self.assertEqual(archive.extractfile("prefix/second").read(), b"second\x00\xff")


if __name__ == "__main__":
    unittest.main()
