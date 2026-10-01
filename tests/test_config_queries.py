from pathlib import Path
import tempfile
import unittest

import gix


class ConfigQueryTests(unittest.TestCase):
    def test_native_options_and_persistent_identity_fallback(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = gix.init(directory, options=gix.OpenOptions.isolated().config_overrides([
                "core.looseCompression=2", "pack.compression=9", "core.editor=example-editor",
                "core.ignoreCase=false", "core.trustCtime=false",
            ]))
            self.assertEqual(repo.loose_compression().level(), 2)
            self.assertEqual(repo.pack_compression().level(), 9)
            self.assertEqual(repo.editor(), "example-editor")
            self.assertFalse(repo.filesystem_options().ignore_case)
            self.assertIn(repo.git_dir_trust(), ("Full", "Reduced"))
            self.assertIsNone(repo.committer())
            fallback = repo.committer_or_set_fallback(b"application\xff", b"app@example.invalid")
            self.assertEqual(fallback.name, b"application\xff")
            self.assertEqual(repo.committer().email, b"app@example.invalid")
            self.assertEqual(repo.committer_or_set_generic_fallback().name, fallback.name)
            if "index" in gix.build_features():
                self.assertFalse(repo.stat_options().trust_ctime)
                self.assertGreater(repo.compute_object_cache_size_for_tree_diffs(repo.index_or_empty()), 0)
            if "attributes" in gix.build_features():
                self.assertEqual(repo.modules_path(), Path(directory) / ".gitmodules")
                self.assertEqual(repo.command_context().git_dir, repo.git_dir())
                self.assertFalse(repo.ignore_pattern_parser().support_precious)


if __name__ == "__main__":
    unittest.main()
