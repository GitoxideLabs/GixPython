from pathlib import Path
import tempfile
import unittest

import gix


@unittest.skipUnless("attributes" in gix.build_features(), "attributes feature disabled")
class AttributeTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name)
        self.repo = gix.init(self.path, options=gix.OpenOptions.isolated())
        self.index = self.repo.index_or_empty()

    def test_attributes_keep_values_and_source_locations(self):
        (self.path / ".gitattributes").write_bytes(b"*.txt text -diff language=python\nraw value=\xff\n")
        stack = self.repo.attributes(self.index)
        platform = stack.at_entry("file.txt")
        self.assertEqual(platform.path(), self.path / "file.txt")
        out = stack.selected_attribute_matches(["text", "diff", "language", "unknown"])
        self.assertTrue(platform.matching_attributes(out))
        with out.iter_selected() as matches:
            result = {match.name: match for match in matches}
        self.assertEqual(result["text"].state, "set")
        self.assertEqual(result["diff"].state, "unset")
        self.assertEqual(result["language"].value, b"python")
        self.assertEqual(result["unknown"].state, "unspecified")
        self.assertEqual(result["language"].source, self.path / ".gitattributes")
        self.assertEqual(result["language"].sequence_number, 1)
        self.assertEqual(result["language"].pattern, b"*.txt")
        self.assertTrue(out.is_done())
        self.assertFalse(platform.is_excluded())
        all_attributes = stack.attribute_matches()
        stack.at_path(Path("raw")).matching_attributes(all_attributes)
        self.assertEqual(next(all_attributes.iter()).value, b"\xff")
        self.assertGreater(stack.statistics()["platforms"], 0)
        stack.take_statistics()
        self.assertEqual(stack.statistics()["platforms"], 0)

    def test_ignore_overrides_and_capability_checks(self):
        (self.path / ".gitignore").write_bytes(b"*.log\n!keep.log\n")
        overrides = gix.IgnoreSearch.from_overrides(["override"])
        stack = self.repo.excludes(self.index, overrides)
        ignored = stack.at_entry("a.log")
        self.assertTrue(ignored.is_excluded())
        self.assertEqual(ignored.excluded_kind(), "Expendable")
        match = ignored.matching_exclude_pattern()
        self.assertEqual(match.pattern, b"*.log")
        self.assertFalse(match.is_negative())
        self.assertFalse(stack.at_entry("keep.log").is_excluded())
        self.assertTrue(stack.at_entry("override").is_excluded())
        with self.assertRaises(ValueError):
            stack.attribute_matches()
        attributes = self.repo.attributes_only(self.index)
        with self.assertRaises(ValueError):
            attributes.at_entry("a.log").is_excluded()

    def test_index_only_sources_work_without_worktree_files(self):
        blob = self.repo.write_blob(b"*.txt from_index\n")
        ignore = self.repo.write_blob(b"*.log\n")
        with self.repo.empty_tree().edit() as editor:
            editor.upsert(".gitattributes", "blob", blob)
            editor.upsert(".gitignore", "blob", ignore)
            index = self.repo.index_from_tree(editor.write())
        stack = self.repo.attributes(index, "id_mapping", "id_mapping")
        out = stack.attribute_matches()
        self.assertTrue(stack.at_entry("file.txt").matching_attributes(out))
        self.assertEqual([match.name for match in out.iter()], ["from_index"])
        self.assertTrue(stack.at_entry("file.log").is_excluded())
        with self.assertRaises(ValueError):
            self.repo.attributes(index, "invalid")


if __name__ == "__main__":
    unittest.main()
