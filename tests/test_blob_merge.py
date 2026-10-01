"""Native prepared blob merges retain resources and driver selection."""
import gc
import os
from pathlib import Path
import tempfile
import unittest

import gix


@unittest.skipUnless("merge" in gix.build_features(), "merge disabled")
class BlobMergeTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name)
        # Native Windows attribute lookup requires a UTF-8 filesystem path.
        self.resource_path = b"file-\xc3\xbf" if os.name == "nt" else b"file-\xff"
        self.repo = gix.init(self.path / "repo", options=gix.OpenOptions.isolated())

    def cache(self, repo=None, ancestor=b"a\nb\nc\n", current=b"A\nb\nc\n", other=b"a\nb\nC\n"):
        repo = repo or self.repo
        cache = repo.merge_resource_cache()
        ids = {}
        for kind, data in [("ancestor", ancestor), ("current", current), ("other", other)]:
            ids[kind] = repo.write_blob(data)
            cache.set_resource(ids[kind], 0o100644, self.resource_path, kind)
        return cache, ids

    def test_nonconflicting_merge_and_native_pick_to_object_for_all_hashes(self):
        for kind in gix.HashKind.all():
            with self.subTest(hash=str(kind)):
                repo = gix.init(self.path / str(kind), object_hash=kind, options=gix.OpenOptions.isolated())
                cache, ids = self.cache(repo)
                progress = gix.Progress()
                token = gix.CancellationToken()
                with cache.prepare_merge(progress=progress, cancel=token) as prepared:
                    self.assertEqual(progress.state, "succeeded")
                    token.cancel()  # The preparation's cancellation scope is complete.
                    self.assertEqual(prepared.current.id, ids["current"])
                    self.assertEqual(prepared.current.rela_path, self.resource_path)
                    self.assertEqual(prepared.current.data, b"A\nb\nc\n")
                    self.assertEqual(prepared.ancestor.as_slice(), b"a\nb\nc\n")
                    self.assertEqual(prepared.other.kind, "Buffer")
                    self.assertEqual(prepared.other.size, 6)
                    self.assertEqual(prepared.driver, "text")
                    self.assertEqual(prepared.configured_driver(), "text")
                    out, pick, resolution = prepared.merge()
                    self.assertEqual((out, pick, resolution), (b"A\nb\nC\n", "Buffer", "Complete"))
                    self.assertIsNone(prepared.buffer_by_pick(pick))
                    merged = prepared.id_by_pick(pick, out)
                    self.assertEqual(repo.find_blob(merged).data, out)
                    self.assertEqual(prepared.builtin_merge("text"), (out, pick, resolution))
                self.assertEqual(progress.state, "succeeded")

    def test_prepared_borrow_blocks_cache_mutation_and_resource_keeps_it_alive(self):
        cache, ids = self.cache()
        prepared = cache.prepare_merge()
        current = prepared.current
        del prepared
        gc.collect()
        self.assertEqual(current.id, ids["current"])
        self.assertEqual(current.data, b"A\nb\nc\n")
        with self.assertRaises(RuntimeError):
            cache.set_resource(ids["current"], 0o100644, "other-path", "current")
        del current
        gc.collect()
        cache.set_resource(ids["current"], 0o100644, "other-path", "current")
        with cache.prepare_merge() as prepared:
            resource = prepared.current
            self.assertEqual(resource.rela_path, b"other-path")
        with self.assertRaises(RuntimeError):
            resource.data

    def test_conflict_options_labels_and_builtin_resolution(self):
        cache, _ = self.cache(ancestor=b"base\n", current=b"ours\n", other=b"theirs\n")
        opts = gix.BlobMergeOptions(conflict_style="diff3", marker_size=9, diff_algorithm="histogram")
        labels = gix.MergeLabels(ancestor=b"base-\xff", current=b"ours-\xff", other=b"theirs-\xff")
        with cache.prepare_merge(opts) as prepared:
            self.assertEqual(prepared.options, opts)
            out, pick, resolution = prepared.merge(labels)
            self.assertEqual((pick, resolution), ("Buffer", "Conflict"))
            self.assertIn(b"<<<<<<<<< ours-\xff\n", out)
            self.assertIn(b"||||||||| base-\xff\n", out)
            self.assertIn(b">>>>>>>>> theirs-\xff\n", out)
            prepared.options = gix.BlobMergeOptions(conflict="theirs")
            out, _, resolution = prepared.merge()
            self.assertEqual(out, b"theirs\n")
            self.assertEqual(resolution, "CompleteWithAutoResolvedConflict")
            prepared.driver = "union"
            out, _, resolution = prepared.merge()
            self.assertIn(b"ours\n", out)
            self.assertIn(b"theirs\n", out)
            self.assertNotIn(b"<<<<<<<", out)
        for kwargs in [{"marker_size": 0}, {"conflict_style": "unknown"}, {"resolve_binary_with": "both"}]:
            with self.assertRaises(ValueError):
                gix.BlobMergeOptions(**kwargs)

    def test_binary_missing_and_too_large_resources_preserve_pick_semantics(self):
        cache, ids = self.cache(ancestor=b"base\0", current=b"ours\0", other=b"theirs\0")
        with cache.prepare_merge() as prepared:
            out, pick, resolution = prepared.merge()
            self.assertEqual((out, pick, resolution), (b"", "Ours", "Conflict"))
            self.assertEqual(prepared.buffer_by_pick(pick), b"ours\0")
            self.assertEqual(prepared.id_by_pick(pick, out), ids["current"])
            prepared.options = gix.BlobMergeOptions(resolve_binary_with="theirs")
            self.assertEqual(prepared.merge()[1:], ("Theirs", "CompleteWithAutoResolvedConflict"))
        cache.large_file_threshold_bytes = 1
        cache.set_resource(ids["current"], 0o100644, "file", "current")
        with cache.prepare_merge() as prepared:
            self.assertEqual(prepared.current.kind, "TooLarge")
            self.assertEqual(prepared.current.size, 5)
            self.assertIsNone(prepared.current.data)
            self.assertIsNone(prepared.current.as_slice())
            with self.assertRaises(ValueError):
                prepared.buffer_by_pick("Ours")
            self.assertEqual(prepared.id_by_pick("Ours", b""), ids["current"])
        cache.set_resource(self.repo.object_hash().null(), 0o100644, "missing", "current")
        with cache.prepare_merge() as prepared:
            self.assertEqual(prepared.current.kind, "Missing")
            self.assertIsNone(prepared.current.data)
            self.assertEqual(prepared.current.as_slice(), b"")
            self.assertIsNone(prepared.id_by_pick("Ours", b""))

    def test_worktree_roots_and_attribute_driver_configuration(self):
        path = self.path / "repo"
        (path / "file").write_bytes(b"worktree\r\n")
        (path / ".gitattributes").write_bytes(b"file text eol=lf merge=binary conflict-marker-size=12\n")
        cache = self.repo.merge_resource_cache(current_root=path)
        self.assertEqual(cache.filter_mode, "to_git")
        cache.filter_mode = "renormalize"
        self.assertEqual(cache.filter_mode, "renormalize")
        self.assertIsNone(cache.default_driver)
        cache.default_driver = b"union"
        self.assertEqual(cache.default_driver, b"union")
        null = self.repo.object_hash().null()
        cache.set_resource(null, 0o100644, "file", "current")
        cache.set_resource(self.repo.write_blob(b"base\n"), 0o100644, "file", "ancestor")
        cache.set_resource(self.repo.write_blob(b"other\n"), 0o100644, "file", "other")
        with cache.prepare_merge() as prepared:
            self.assertEqual(prepared.current.data, b"worktree\n")
            self.assertEqual(prepared.driver, "binary")
            self.assertEqual(prepared.options.marker_size, 12)
            self.assertEqual(self.repo.find_blob(prepared.id_by_pick("Ours", b"")).data, b"worktree\n")

    @unittest.skipUnless(os.name == "posix", "fixture uses a POSIX shell driver")
    def test_configured_external_driver_execution_and_failure(self):
        path = self.path / "driver"
        repo = gix.init(path, options=gix.OpenOptions.isolated().config_overrides([
            "merge.default=take-other", "merge.take-other.name=Keep their content", "merge.take-other.driver=cat %B > %A", "merge.take-other.recursive=binary"
        ]))
        cache, _ = self.cache(repo)
        driver = cache.drivers()[0]
        self.assertEqual(driver.name, b"take-other")
        self.assertEqual(driver.display_name, b"take-other")  # Native gix currently uses the configured key name.
        self.assertEqual(driver.command, b"cat %B > %A")
        self.assertEqual(driver.recursive, b"binary")
        with cache.prepare_merge() as prepared:
            self.assertEqual(prepared.driver, 0)
            self.assertEqual(prepared.configured_driver().command, driver.command)
            self.assertEqual(prepared.merge(), (b"a\nb\nC\n", "Buffer", "Complete"))
        with cache.prepare_merge(gix.BlobMergeOptions(is_virtual_ancestor=True)) as prepared:
            self.assertEqual(prepared.driver, "binary")
            self.assertEqual(prepared.options.resolve_binary_with, "ours")
        failing = gix.init(self.path / "failing", options=gix.OpenOptions.isolated().config_overrides([
            "merge.default=fail", "merge.fail.driver=exit 17"
        ]))
        bad_cache, _ = self.cache(failing)
        with bad_cache.prepare_merge() as prepared:
            with self.assertRaises(gix.Error):
                prepared.merge()
            self.assertEqual(prepared.current.data, b"A\nb\nc\n")

    def test_driver_descriptors_preserve_raw_configuration_bytes(self):
        config = Path(self.repo.git_dir()) / "config"
        with config.open("ab") as stream:
            stream.write(b'\n[merge "raw-\xff"]\ndriver = echo \xff\nrecursive = raw-\xfe\n')
        repo = gix.open_opts(self.path / "repo", gix.OpenOptions.isolated())
        driver = repo.merge_resource_cache().drivers()[0]
        self.assertEqual(driver.name, b"raw-\xff")
        self.assertEqual(driver.display_name, b"raw-\xff")
        self.assertEqual(driver.command, b"echo \xff")
        self.assertEqual(driver.recursive, b"raw-\xfe")

    def test_invalid_resources_and_cancelled_preparation_report_errors(self):
        cache = self.repo.merge_resource_cache()
        with self.assertRaises(gix.Error):
            cache.prepare_merge()
        cache, ids = self.cache()
        with self.assertRaises(gix.Error):
            cache.set_resource(ids["current"], 0o040000, "tree", "current")
        token = gix.CancellationToken()
        token.cancel()
        with self.assertRaises(gix.CancelledError):
            cache.prepare_merge(cancel=token)
        with cache.prepare_merge() as prepared:
            self.assertEqual(prepared.current.id, ids["current"])


if __name__ == "__main__":
    unittest.main()
