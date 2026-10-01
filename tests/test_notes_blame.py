from pathlib import Path
import tempfile
import unittest
import gix


class NotesAndBlameTests(unittest.TestCase):
    def repository(self, directory):
        return gix.init(directory, options=gix.OpenOptions.isolated().config_overrides([
            "user.name=Example", "user.email=example@example.com",
        ]))

    @unittest.skipUnless("notes" in gix.build_features(), "notes disabled")
    def test_native_notes_cache_and_mutations(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = self.repository(directory)
            oid = repo.write_blob(b"annotated")
            with repo.notes() as notes:
                self.assertEqual(notes.default_ref(), b"refs/notes/commits")
                self.assertEqual(list(notes.refs()), [b"refs/notes/commits"])
                self.assertEqual(notes.get(oid), [])
                self.assertIsNone(notes.replace("commits", oid, b"first\xff"))
                first = notes.get(oid)[0]
                self.assertEqual(first.blob.data, b"first\xff")
                self.assertEqual(first.reference, b"refs/notes/commits")
                self.assertEqual(notes.replace_at_ref("refs/notes/commits", oid, b"second"), first.blob.id)
                self.assertEqual(notes.get(oid)[0].blob.data, b"second")
                self.assertIsNotNone(notes.remove("commits", oid))
                self.assertEqual(notes.get(oid), [])

    @unittest.skipUnless("blame" in gix.build_features(), "blame disabled")
    def test_blame_ranges_and_lines(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = self.repository(directory)
            with repo.empty_tree().edit() as editor:
                editor.upsert(b"file", "blob", repo.write_blob(b"one\ntwo\n"))
                tree = editor.write()
            first = repo.commit("HEAD", "first", tree, [])
            with repo.find_tree(tree).edit() as editor:
                editor.upsert(b"file", "blob", repo.write_blob(b"one\nchanged\n"))
                tree = editor.write()
            second = repo.commit("HEAD", "second", tree, [first])
            outcome = repo.blame_file(b"file", second)
            self.assertEqual(outcome.blob, b"one\nchanged\n")
            self.assertEqual([e.commit_id for e in outcome.entries], [first, second])
            self.assertEqual([line for _, lines in outcome.entries_with_lines() for line in lines], [b"one\n", b"changed\n"])
            partial = repo.blame_file("file", second, gix.BlameOptions(ranges=[(2, 2)]))
            self.assertEqual([e.commit_id for e in partial.entries], [second])
            with self.assertRaises(ValueError):
                gix.BlameOptions(ranges=[(0, 1)])


if __name__ == "__main__":
    unittest.main()
