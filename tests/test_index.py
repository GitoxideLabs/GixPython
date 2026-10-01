import concurrent.futures
from pathlib import Path
import tempfile
import unittest

import gix


@unittest.skipUnless("index" in gix.build_features(), "index feature disabled")
class IndexTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name)
        self.repo = gix.init(self.path, options=gix.OpenOptions.isolated())

    def test_index_from_tree_round_trips_for_each_hash(self):
        for kind in gix.HashKind.all():
            with self.subTest(hash=str(kind)):
                repo = gix.init(self.path / str(kind), object_hash=kind, options=gix.OpenOptions.isolated())
                self.assertIsNone(repo.try_index())
                self.assertEqual(len(repo.index_or_load_from_head_or_empty()), 0)
                blob = repo.write_blob(b"file")
                with repo.empty_tree().edit() as editor:
                    editor.upsert(b"dir/a\xff", "blob", blob)
                    editor.upsert("run", "exe", blob)
                    index = repo.index_from_tree(editor.write())
                self.assertEqual(index.object_hash(), kind)
                self.assertEqual(len(index), 2)
                self.assertEqual(index.entry(0).path(), b"dir/a\xff")
                self.assertEqual(index.entry_by_path("run").mode, 0o100755)
                self.assertTrue(index.path_is_directory("dir"))
                self.assertEqual(index.prefixed_entries_range("dir/"), (0, 1))
                index.write()
                index.verify_integrity()
                self.assertEqual(repo.open_index().checksum(), index.checksum())
                self.assertEqual([entry.path() for entry in repo.index()], [b"dir/a\xff", b"run"])

    def test_entry_mutation_does_not_change_an_existing_iterator(self):
        index = self.repo.index_or_empty()
        before = self.repo.write_blob(b"before")
        after = self.repo.write_blob(b"after")
        index.dangerously_push_entry(gix.IndexStat(), before, 0, 0o100644, "file")
        entries = index.entries()
        entry = index.entry_mut_by_path_and_stage("file", 0)
        entry.id = after
        entry.mode = 0o100755
        entry.stat = gix.IndexStat(size=5)
        self.assertEqual(index.entry(0).id, after)
        self.assertEqual(index.entry(0).stat.size, 5)
        self.assertEqual(next(entries).id, before)
        entries.close()
        index.write()
        self.assertEqual(self.repo.open_index().entry(0).id, after)
        self.assertEqual(index.remove_entry_at_index(0).id, after)
        with self.assertRaises(KeyError):
            entry.id = before

    def test_conflict_stages_and_write_verification(self):
        index = self.repo.index_or_empty()
        oid = self.repo.write_blob(b"content")
        for stage in (3, 1, 2):
            index.dangerously_push_entry(gix.IndexStat(), oid, stage << 12, 0o100644, "conflict")
        with self.assertRaises(gix.Error):
            index.write()
        index.sort_entries()
        index.verify_entries()
        self.assertEqual(index.entry_range("conflict"), (0, 3))
        self.assertEqual(index.entry_by_path("conflict").stage(), 2)
        self.assertEqual([entry.stage() for entry in index], [1, 2, 3])
        index.write()
        self.assertEqual(self.repo.open_index().entry_by_path_and_stage("conflict", 3).id, oid)
        with self.assertRaises(IndexError):
            index.entry(10)
        with self.assertRaises(ValueError):
            index.entry_by_path_and_stage("conflict", 4)

    def test_shared_snapshots_and_concurrent_reads(self):
        first = self.repo.index_or_empty()
        oid = self.repo.write_blob(b"content")
        first.dangerously_push_entry(gix.IndexStat(), oid, 0, 0o100644, "file")
        first.write()
        snapshot = self.repo.index()
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as executor:
            self.assertEqual(list(executor.map(lambda _: snapshot.entry_by_path("file").id, range(32))), [oid] * 32)
        replacement = self.repo.write_blob(b"new")
        mutable = self.repo.index()
        mutable.entry_mut_by_path_and_stage("file", 0).id = replacement
        mutable.write()
        self.assertEqual(snapshot.entry(0).id, oid)
        self.assertEqual(self.repo.open_index().entry(0).id, replacement)


if __name__ == "__main__":
    unittest.main()
