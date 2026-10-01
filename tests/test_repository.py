import concurrent.futures
from pathlib import Path
import tempfile
import unittest

import gix


class RepositoryTests(unittest.TestCase):
    def test_hashes_open_discover_and_shared_reads(self):
        for kind in gix.HashKind.all():
            with self.subTest(hash=str(kind)), tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "repo"
                repo = gix.init(path, object_hash=kind, options=gix.OpenOptions.isolated())
                self.assertEqual(repo.object_hash(), kind)
                self.assertFalse(repo.is_bare())
                self.assertEqual(Path(repo.workdir()), path)
                self.assertEqual(gix.open_opts(path, gix.OpenOptions.isolated()).object_hash(), kind)
                child = path / "a" / "b"
                child.mkdir(parents=True)
                self.assertEqual(Path(gix.discover_opts(child, gix.OpenOptions.isolated()).git_dir()), path / ".git")
                oid = repo.write_blob(b"\x00binary\xff")
                with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
                    values = list(pool.map(lambda _: repo.find_blob(oid).data, range(128)))
                self.assertEqual(values, [b"\x00binary\xff"] * 128)
                self.assertEqual(len(bytes(oid)), kind.len_in_bytes())
                self.assertEqual(gix.ObjectId(str(oid)), oid)

    def test_mutations_and_object_memory_persist(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = gix.init(Path(directory) / "repo", options=gix.OpenOptions.isolated())
            memory = repo.with_object_memory()
            oid = memory.write_blob(b"only in memory")
            self.assertEqual(memory.find_blob(oid).data, b"only in memory")
            self.assertFalse(repo.has_object(oid))
            self.assertEqual(memory.with_object_memory().find_blob(oid).data, b"only in memory")
            previous = repo.workdir()
            changed = Path(directory) / "worktree"
            changed.mkdir()
            self.assertEqual(repo.set_workdir(changed), previous)
            self.assertEqual(Path(repo.workdir()), changed)
            self.assertIs(repo.reload(), repo)
            self.assertEqual(repo.workdir(), previous)

    def test_bad_ids_and_mismatched_hashes(self):
        with self.assertRaises(ValueError):
            gix.ObjectId("not an object id")
        if len(gix.HashKind.all()) < 2:
            return
        with tempfile.TemporaryDirectory() as directory:
            repo = gix.init(directory, options=gix.OpenOptions.isolated())
            with self.assertRaises(ValueError):
                repo.find_object(gix.HashKind.SHA256.null())


if __name__ == "__main__":
    unittest.main()
