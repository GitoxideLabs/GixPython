from pathlib import Path
import tempfile
import unittest

import gix


@unittest.skipUnless("attributes" in gix.build_features(), "attributes feature disabled")
class PathspecTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name)
        self.repo = gix.init(self.path, options=gix.OpenOptions.isolated())
        blob = self.repo.write_blob(b"content")
        with self.repo.empty_tree().edit() as editor:
            for name in (b"dir/a\xff.txt", b"dir/skip.txt", b"other"):
                editor.upsert(name, "blob", blob)
            self.index = self.repo.index_from_tree(editor.write())

    def test_matching_and_lazy_index_paths(self):
        spec = self.repo.pathspec(False, [b"dir/*.txt", b":(exclude)dir/skip.txt"], False, self.index)
        self.assertTrue(spec.is_included(b"dir/a\xff.txt", False))
        self.assertFalse(spec.is_included("dir/skip.txt", False))
        match = spec.pattern_matching_relative_path("dir/skip.txt", False)
        self.assertTrue(match.is_excluded())
        self.assertTrue(match.pattern.is_excluded())
        with spec.search().patterns() as patterns:
            self.assertEqual(len(list(patterns)), 2)
        with spec.index_entries_with_paths(self.index) as entries:
            path, entry = next(entries)
            self.assertEqual(path, b"dir/a\xff.txt")
            self.assertEqual(entry.path(), path)
            self.assertEqual(list(entries), [])
        absent = self.repo.pathspec(False, ["missing/file"], False, self.index)
        self.assertIsNone(absent.index_entries_with_paths(self.index))

    def test_attribute_requirements_use_native_matching(self):
        (self.path / ".gitattributes").write_bytes(b"*.txt language=python\n")
        spec = self.repo.pathspec(False, [":(attr:language=python)*"], False, self.index)
        self.assertTrue(spec.is_included("dir/skip.txt", False))
        self.assertFalse(spec.is_included("other", False))
        self.assertIsInstance(self.repo.pathspec_defaults().literal, bool)
        with self.assertRaises(ValueError):
            self.repo.pathspec(False, [], False, self.index, "invalid")


if __name__ == "__main__":
    unittest.main()
