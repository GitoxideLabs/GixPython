"""Native object round trips, lazy ownership, editing, and interruption."""

import concurrent.futures
import io
from pathlib import Path
import tempfile
import unittest
import zlib

import gix


class ObjectTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name) / "repo"
        self.repo = gix.init_bare(self.path, options=gix.OpenOptions.isolated())
        self.signature = gix.Signature(b"A\xff", b"a@example.com", 1700000000, 3600)

    def make_tree(self, repo=None):
        repo = repo or self.repo
        blob = repo.write_blob(b"binary\x00\xff")
        tree = repo.find_tree(repo.write_object("tree", b"100644 a\xff\0" + bytes(blob)))
        return blob, tree

    def test_all_hashes_round_trip_bytes_and_headers(self):
        for kind in gix.HashKind.all():
            with self.subTest(hash=str(kind)):
                repo = gix.init_bare(Path(self.directory.name) / str(kind), object_hash=kind, options=gix.OpenOptions.isolated())
                blob, tree = self.make_tree(repo)
                self.assertEqual(repo.find_blob(blob).data, b"binary\x00\xff")
                self.assertEqual(repo.write_blob_stream(io.BytesIO(b"binary\x00\xff")), blob)
                self.assertEqual(tree.id.kind(), kind)
                header = repo.find_header(blob)
                self.assertEqual((header.kind(), header.size(), header.num_deltas()), ("blob", 8, None))
                self.assertEqual(repo.try_find_header(tree).kind(), "tree")
                self.assertIsNone(repo.try_find_object(kind.null()))
                self.assertIsNone(repo.try_find_header(kind.null()))
                self.assertFalse(repo.has_object(kind.null()))
                self.assertEqual(repo.empty_tree().id, kind.empty_tree())
                self.assertEqual(repo.empty_blob().id, kind.empty_blob())
                self.assertEqual(list(repo.empty_tree()), [])
                self.assertEqual(tree.find_entry(b"a\xff").filename(), b"a\xff")
                self.assertEqual(tree.lookup_entry([b"a\xff"]).object_id(), blob)
                self.assertEqual(tree.decode().entries[0].filename(), b"a\xff")

    def test_commit_creation_parents_and_compare_and_swap(self):
        _, tree = self.make_tree()
        first = self.repo.commit_as(self.signature, self.signature, "HEAD", "first\n", tree)
        second = self.repo.commit_as(self.signature, self.signature, "HEAD", "second\n", tree, [first])
        commit = self.repo.find_commit(second)
        self.assertEqual(list(commit.parent_ids()), [first])
        self.assertEqual(commit.tree_id(), tree.id)
        self.assertEqual(commit.tree().data, tree.data)
        self.assertEqual(commit.message_raw(), b"second\n")
        self.assertEqual(commit.author().name, b"A\xff")
        self.assertEqual(commit.committer().email, b"a@example.com")
        self.assertEqual((commit.time().seconds, commit.time().offset), (1700000000, 3600))
        decoded = commit.decode()
        self.assertEqual(decoded.tree, str(tree.id).encode())
        self.assertEqual(decoded.parents, (str(first).encode(),))
        self.assertEqual(decoded.message, b"second\n")
        self.assertIsNone(decoded.encoding)
        self.assertEqual(decoded.extra_headers, ())
        with self.assertRaises(gix.Error):
            self.repo.commit_as(self.signature, self.signature, "HEAD", "stale", tree, [first])
        self.assertEqual(self.repo.head_id(), second)
        draft = self.repo.new_commit_as(self.signature, self.signature, "unreferenced", tree, [second])
        self.assertEqual(list(draft.parent_ids()), [second])
        self.assertEqual(self.repo.head_id(), second)
        if "revision" in gix.build_features():
            self.assertEqual(self.repo.find_commit("HEAD").id, second)
            self.assertEqual(self.repo.find_tree(b"HEAD^{tree}").id, tree.id)
            with self.assertRaises(ValueError):
                self.repo.find_tree("HEAD")

    def test_tags_preserve_native_creation_constraints(self):
        commit = self.repo.new_commit_as(self.signature, self.signature, "root", self.repo.empty_tree())
        reference = self.repo.tag("v1", commit, "commit", self.signature, "release\n", gix.PreviousValue.MustNotExist)
        tag = self.repo.find_tag(reference.id())
        self.assertEqual(tag.target_id(), commit.id)
        self.assertEqual(tag.tagger().name, b"A\xff")
        decoded = tag.decode()
        self.assertEqual((decoded.name, decoded.target_kind, decoded.message), (b"v1", "commit", b"release\n"))
        self.assertEqual(decoded.target, str(commit.id).encode())
        self.assertIsNone(decoded.signature)
        self.assertEqual(tag.into_object().peel_to_commit().id, commit.id)
        self.assertEqual(tag.into_object().peel_to_tree().id, commit.tree_id())
        self.assertEqual(tag.into_object().peel_tags_to_end().id, commit.id)
        with self.assertRaises(gix.Error):
            self.repo.tag("v1", commit, "commit", None, "duplicate", gix.PreviousValue.MustNotExist)

    def test_objects_and_iterators_own_their_repository_and_bytes(self):
        blob, tree = self.make_tree()
        entries = iter(tree)
        del tree
        del self.repo
        entry = next(entries)
        self.assertEqual(entry.object_id(), blob)
        self.assertEqual(entry.object().data, b"binary\x00\xff")
        with self.assertRaises(StopIteration):
            next(entries)
        self.assertEqual(entry.filename(), b"a\xff")

    def test_iterator_progress_and_cancellation_are_lazy(self):
        _, tree = self.make_tree()
        progress = gix.Progress()
        token = gix.CancellationToken()
        entries = tree.iter(progress=progress, cancel=token)
        self.assertEqual(progress.state, "idle")
        next(entries)
        self.assertEqual(progress.state, "running")
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
            executor.submit(token.cancel).result(timeout=5)
        with self.assertRaises(gix.CancelledError):
            next(entries)
        self.assertEqual(progress.state, "cancelled")
        self.assertTrue(token.cancelled)
        self.assertEqual(list(entries), [])
        unused_progress = gix.Progress()
        unused = tree.iter(progress=unused_progress, cancel=token)
        with self.assertRaises(gix.CancelledError):
            next(unused)
        self.assertEqual(unused_progress.state, "idle")

    def test_progress_reuse_and_context_manager_close(self):
        _, tree = self.make_tree()
        progress = gix.Progress()
        with tree.iter(progress=progress) as entries:
            next(entries)
            with self.assertRaisesRegex(RuntimeError, "progress is already in use"):
                next(tree.iter(progress=progress))
        self.assertEqual(progress.state, "cancelled")
        self.assertEqual(list(entries), [])
        self.assertEqual(len(list(tree.iter(progress=progress))), 1)
        self.assertEqual(progress.state, "succeeded")

    def test_tree_editor_retains_native_state_and_can_recover_after_error(self):
        blob = self.repo.write_blob(b"file")
        with self.repo.edit_tree(self.repo.empty_tree()) as editor:
            self.assertIs(editor.upsert(b"dir/a\xff", "blob", blob), editor)
            self.assertEqual(editor.get(b"dir/a\xff").object_id(), blob)
            with self.assertRaises(gix.Error):
                editor.remove_leaf("dir")
            first = editor.write()
            self.assertIs(editor.upsert("run", "exe", blob), editor)
            second = editor.write()
            self.assertNotEqual(first, second)
            tree = self.repo.find_tree(second)
            self.assertEqual(tree.lookup_entry(["dir", b"a\xff"]).object_id(), blob)
            self.assertEqual(tree.lookup_entry_by_path("run").mode(), 0o100755)
            self.assertIs(editor.remove("dir"), editor)
            self.assertIsNone(self.repo.find_tree(editor.write()).find_entry("dir"))
            self.assertIs(editor.set_root(self.repo.empty_tree()), editor)
            self.assertEqual(editor.write(), self.repo.empty_tree().id)
        with self.assertRaisesRegex(RuntimeError, "closed"):
            editor.write()
        with tree.edit() as same:
            self.assertEqual(same.write(), second)

    def test_corrupt_objects_are_errors_not_absence(self):
        oid = self.repo.write_blob(b"temporary")
        loose = self.path / "objects" / str(oid)[:2] / str(oid)[2:]
        loose.chmod(0o600)
        loose.write_bytes(zlib.compress(b"tree 3\0bad"))
        corrupt = self.repo.find_tree(oid)
        with self.assertRaises(gix.Error):
            next(iter(corrupt))
        with self.assertRaises(gix.Error):
            corrupt.find_entry("absent")
        with self.assertRaises(gix.Error):
            corrupt.decode()
        loose.write_bytes(b"not zlib")
        with self.assertRaises(gix.Error):
            self.repo.try_find_object(oid)
        with self.assertRaises(gix.Error):
            self.repo.try_find_header(oid)
        with self.assertRaises(gix.Error):
            self.repo.write_object("tree", b"bad")


if __name__ == "__main__":
    unittest.main()
