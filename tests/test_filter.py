import concurrent.futures
import gc
import os
from pathlib import Path
import shlex
import sys
import tempfile
import unittest

import gix


@unittest.skipUnless("attributes" in gix.build_features(), "attributes feature disabled")
class FilterTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name)
        self.repo = gix.init(self.path, options=gix.OpenOptions.isolated())

    def test_native_conversion_stream_and_pipeline_reuse(self):
        (self.path / ".gitattributes").write_bytes(b"*.txt text eol=crlf\n")
        pipeline, index = self.repo.filter_pipeline()
        with pipeline:
            stream = pipeline.convert_to_worktree(b"a\nb\n", "file.txt")
            self.assertTrue(stream.is_changed())
            self.assertFalse(stream.is_delayed())
            self.assertEqual(stream.read(0), b"")
            self.assertEqual(stream.read(2), b"a\r")
            with self.assertRaises(RuntimeError):
                pipeline.convert_to_git(b"x\r\n", "file.txt", index)
            self.assertEqual(stream.read(), b"\nb\r\n")
            self.assertEqual(stream.read(), b"")
            stream.close()
            self.assertTrue(stream.closed)
            with self.assertRaises(ValueError):
                stream.read()
            with pipeline.convert_to_git(b"x\r\n", "file.txt", index) as stream:
                self.assertEqual(stream.read(), b"x\n")
            abandoned = pipeline.convert_to_worktree(b"discarded\n", "file.txt")
            del abandoned
            gc.collect()
            with pipeline.convert_to_worktree(b"next\n", "file.txt") as stream:
                self.assertEqual(stream.read(), b"next\r\n")

    def test_worktree_file_context_and_independent_parallel_pipelines(self):
        (self.path / ".gitattributes").write_bytes(b"*.txt text eol=crlf\n")
        (self.path / "file.txt").write_bytes(b"worktree\r\n")
        pipeline, index = self.repo.filter_pipeline()
        with pipeline:
            oid, kind, metadata = pipeline.worktree_file_to_object("file.txt", index)
            self.assertEqual(self.repo.find_blob(oid).data, b"worktree\n")
            self.assertEqual(kind, "Blob")
            self.assertEqual(metadata.len(), 10)
            self.assertTrue(metadata.is_file())
            self.assertFalse(metadata.is_symlink())
            self.assertIsInstance(metadata.modified(), float)
            self.assertIsNone(pipeline.worktree_file_to_object("missing", index))
            context = pipeline.driver_context_mut()
            context.ref_name = b"refs/heads/example"
            context.blob = oid
            context.treeish = self.repo.empty_tree().id
            self.assertEqual(context.ref_name, b"refs/heads/example")
            self.assertEqual(context.blob, oid)
            self.assertEqual(context.treeish, self.repo.empty_tree().id)

        def convert(_):
            pipeline, index = self.repo.filter_pipeline()
            with pipeline, pipeline.convert_to_git(b"parallel\r\n", "file.txt", index) as stream:
                return stream.read()

        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as executor:
            self.assertEqual(list(executor.map(convert, range(16))), [b"parallel\n"] * 16)

    def test_bare_index_sources_and_ident_support_each_hash(self):
        for kind in gix.HashKind.all():
            with self.subTest(hash=str(kind)):
                repo = gix.init_bare(self.path / str(kind), object_hash=kind, options=gix.OpenOptions.isolated())
                attributes = repo.write_blob(b"*.txt ident\n")
                with repo.empty_tree().edit() as editor:
                    editor.upsert(".gitattributes", "blob", attributes)
                    tree = editor.write()
                pipeline, index = repo.filter_pipeline(tree)
                with pipeline:
                    data = b"$Id$\n"
                    expected_id = str(repo.write_blob(data)).encode()
                    with pipeline.convert_to_worktree(data, "file.txt") as stream:
                        expanded = stream.read()
                    self.assertEqual(expanded, b"$Id: " + expected_id + b"$\n")
                    with pipeline.convert_to_git(expanded, "file.txt", index) as stream:
                        self.assertEqual(stream.read(), data)

    @unittest.skipIf(os.name == "nt", "test command uses POSIX shell quoting")
    def test_configured_native_filter_helpers(self):
        (self.path / ".gitattributes").write_bytes(b"*.custom filter=uppercase\n")
        script = self.path / "uppercase.py"
        script.write_text("import sys\nsys.stdout.buffer.write(sys.stdin.buffer.read().upper())\n")
        command = shlex.quote(sys.executable) + " " + shlex.quote(str(script))
        with self.repo.config_snapshot_mut() as config:
            config.set_raw_value("filter.uppercase.smudge", command)
            config.set_raw_value("filter.uppercase.clean", command)
            config.set_raw_value("filter.uppercase.required", "true")
        pipeline, index = self.repo.filter_pipeline()
        with pipeline:
            with pipeline.convert_to_worktree(b"native helper", "file.custom") as stream:
                self.assertEqual(stream.read(), b"NATIVE HELPER")
            with pipeline.convert_to_git(b"second call", "file.custom", index) as stream:
                self.assertEqual(stream.read(), b"SECOND CALL")


if __name__ == "__main__":
    unittest.main()
