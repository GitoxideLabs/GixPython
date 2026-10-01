import os
from pathlib import Path
import tempfile
import unittest

import gix


def init_repo(path, kind=None):
    options = gix.OpenOptions.isolated().config_overrides([
        "user.name=Reference Tester",
        "user.email=refs@example.invalid",
        "init.defaultBranch=main",
        "core.logAllRefUpdates=true",
    ])
    return gix.init(path, options=options, object_hash=kind)


def commits(repo):
    signature = gix.Signature(b"Reference Tester", b"refs@example.invalid", 1234567890)
    tree = repo.empty_tree().id
    first = repo.commit_as(signature, signature, "HEAD", "first", tree)
    second = repo.new_commit_as(signature, signature, "second", tree, [first]).id
    return first, second


class ReferenceTests(unittest.TestCase):
    def test_head_states_for_all_enabled_hashes(self):
        for kind in gix.HashKind.all():
            with self.subTest(hash=str(kind)), tempfile.TemporaryDirectory() as directory:
                repo = init_repo(directory, kind)
                head = repo.head()
                self.assertEqual(head.name(), b"HEAD")
                self.assertTrue(head.is_unborn())
                self.assertFalse(head.is_detached())
                self.assertIsNone(head.id())
                self.assertEqual(head.referent_name(), b"refs/heads/main")
                self.assertEqual(repo.head_name(), b"refs/heads/main")
                self.assertIsNone(repo.head_ref())
                self.assertIsNone(head.try_peel_to_id())
                self.assertEqual(repo.head_tree_id_or_empty(), repo.empty_tree().id)
                with self.assertRaises(gix.Error):
                    repo.head_commit()
                first, second = commits(repo)
                self.assertEqual(repo.head_id(), first)
                self.assertEqual(repo.head_ref().id(), first)
                self.assertEqual(repo.head().try_into_referent().id(), first)
                self.assertEqual(repo.head().peel_to_commit().id, first)
                self.assertEqual(repo.head_commit().id, first)
                self.assertEqual(repo.head_tree_id(), repo.head_tree().id)
                repo.edit_reference(gix.RefEdit.update("HEAD", gix.Target.Object(second), gix.PreviousValue.Any, "detach"))
                head = repo.head()
                self.assertTrue(head.is_detached())
                self.assertIsNone(head.referent_name())
                self.assertIsNone(repo.head_name())
                self.assertEqual(head.id(), second)

    def test_literal_compare_and_swap_and_preparation_rollback(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = init_repo(directory)
            first, second = commits(repo)
            original = repo.reference("refs/heads/topic", first, gix.PreviousValue.MustNotExist, "create")
            # Preserve native idempotent MustNotExist behavior for the same target.
            repo.reference("refs/heads/topic", first, gix.PreviousValue.MustNotExist, "same")
            repo.reference("refs/heads/topic", second, gix.PreviousValue.MustExist, "move")
            with self.assertRaises(gix.Error):
                original.set_target_id(first, "stale")
            with self.assertRaises(gix.Error):
                original.delete()
            with self.assertRaises(TypeError):
                gix.PreviousValue.MustExistAndMatch("HEAD")
            with self.assertRaises(TypeError):
                gix.Target.Object("HEAD")
            edits = [
                gix.RefEdit.update("refs/heads/a-new", gix.Target.Object(first), gix.PreviousValue.MustNotExist, "create"),
                gix.RefEdit.update("refs/heads/topic", gix.Target.Object(first), gix.PreviousValue.MustExistAndMatch(gix.Target.Object(first)), "stale"),
            ]
            with self.assertRaises(gix.Error):
                repo.edit_references(edits)
            self.assertIsNone(repo.try_find_reference("refs/heads/a-new"))
            self.assertEqual(repo.find_reference("topic").id(), second)
            current = repo.find_reference("topic")
            current.set_target_id(first, "move back")
            self.assertEqual(current.id(), first)
            current.delete()
            self.assertIsNone(repo.try_find_reference("topic"))
            with self.assertRaises(ValueError):
                repo.edit_reference(gix.RefEdit("refs/heads/invalid", gix.Change.Delete(gix.PreviousValue.MustNotExist, gix.RefLog.AndReference)))

    def test_symbolic_target_snapshot(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = init_repo(directory)
            first, second = commits(repo)
            name = b"refs/heads/target"
            reference = repo.reference(name, second, gix.PreviousValue.Any, b"raw \xff")
            self.assertEqual(reference.name(), name)
            self.assertEqual(repo.find_reference(name).target(), gix.Target.Object(second))
            expected = gix.Target.Symbolic(b"refs/heads/main")
            repo.edit_reference(gix.RefEdit.update("refs/heads/alias", expected, gix.PreviousValue.Any, "symbolic"))
            alias = repo.find_reference("alias")
            self.assertEqual(alias.target(), expected)
            self.assertIsNone(alias.try_id())
            with self.assertRaises(ValueError):
                alias.id()
            self.assertEqual(alias.follow().id(), first)
            repo.edit_reference(gix.RefEdit.update("refs/heads/alias", gix.Target.Symbolic(name), gix.PreviousValue.Any, "retarget"))
            with self.assertRaises(gix.Error):
                alias.delete()
            self.assertEqual(repo.find_reference("alias").follow_to_object(), second)

    def test_raw_reference_names_when_filesystem_supports_them(self):
        with tempfile.TemporaryDirectory() as directory:
            probe = os.fsencode(directory) + b"/raw-\xff"
            try:
                with open(probe, "wb"):
                    pass
            except OSError as error:
                self.skipTest(f"filesystem rejects non-UTF-8 filenames: {error}")
            finally:
                if os.path.exists(probe):
                    os.unlink(probe)
            repo = init_repo(directory)
            first, _ = commits(repo)
            name = b"refs/heads/raw-\xff"
            ref = repo.reference(name, first, gix.PreviousValue.Any, "raw name")
            self.assertEqual(ref.name(), name)
            self.assertEqual(repo.find_reference(name).id(), first)
            self.assertIn(name, [ref.name() for ref in repo.references().all()])

    def test_lazy_selections_peeling_and_close(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = init_repo(directory)
            first, _ = commits(repo)
            repo.reference("refs/heads/a-early", first, gix.PreviousValue.Any, "create")
            repo.reference("refs/remotes/origin/main", first, gix.PreviousValue.Any, "create")
            repo.tag_reference("light", first, gix.PreviousValue.Any)
            platform = repo.references()
            self.assertEqual([r.name() for r in platform.tags().peeled()], [b"refs/tags/light"])
            self.assertEqual([r.name() for r in platform.remote_branches()], [b"refs/remotes/origin/main"])
            self.assertEqual([r.name() for r in platform.prefixed(b"refs/heads/a")], [b"refs/heads/a-early"])
            broken = Path(repo.git_dir()) / "refs" / "heads" / "z-corrupt"
            broken.write_bytes(b"this is not a reference\n")
            with platform.local_branches() as cursor:
                self.assertEqual(next(cursor).name(), b"refs/heads/a-early")
            self.assertEqual(list(cursor), [])
            with platform.local_branches() as cursor:
                self.assertEqual(next(cursor).name(), b"refs/heads/a-early")
                self.assertEqual(next(cursor).name(), b"refs/heads/main")
                with self.assertRaises(gix.Error):
                    next(cursor)

    def test_reflog_order_missing_and_lazy_parse_errors(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = init_repo(directory)
            first, second = commits(repo)
            ref = repo.reference("refs/heads/topic", first, gix.PreviousValue.Any, b"first \xff")
            ref.set_target_id(second, b"second \xfe")
            log = ref.log_iter()
            forward = list(log.all())
            reverse = list(log.rev())
            self.assertEqual([line.message for line in forward], [b"first \xff", b"second \xfe"])
            self.assertEqual([line.message for line in reverse], [b"second \xfe", b"first \xff"])
            self.assertEqual(forward[0].new_oid, first)
            self.assertTrue(forward[0].previous_oid.is_null())
            self.assertEqual(forward[1].previous_oid, first)
            self.assertEqual(forward[1].new_oid, second)
            self.assertEqual(forward[0].signature.email, b"refs@example.invalid")
            tag = repo.tag_reference("no-log", first, gix.PreviousValue.Any)
            self.assertIsNone(tag.log_iter().all())
            self.assertIsNone(tag.log_iter().rev())
            with (Path(repo.git_dir()) / "logs" / "refs" / "heads" / "topic").open("ab") as stream:
                stream.write(b"malformed\n")
            with log.all() as cursor:
                self.assertEqual(next(cursor).new_oid, first)
                self.assertEqual(next(cursor).new_oid, second)
                with self.assertRaises(gix.Error):
                    next(cursor)

    def test_namespace_mutation_and_branch_protection(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = init_repo(directory)
            first, second = commits(repo)
            self.assertIsNone(repo.namespace())
            self.assertIsNone(repo.set_namespace("tenant"))
            self.assertEqual(repo.namespace(), b"refs/namespaces/tenant/")
            repo.reference("refs/heads/scoped", first, gix.PreviousValue.Any, "create")
            self.assertEqual(repo.find_reference("scoped").id(), first)
            self.assertEqual(repo.clear_namespace(), b"refs/namespaces/tenant/")
            self.assertIsNone(repo.try_find_reference("scoped"))
            with self.assertRaises(gix.Error):
                repo.delete_local_branches(["refs/heads/main"])
            stale = repo.reference("refs/heads/topic", first, gix.PreviousValue.Any, "create")
            repo.reference("refs/heads/topic", second, gix.PreviousValue.Any, "move")
            with self.assertRaises(gix.Error):
                repo.delete_local_branches_if_unchanged([(stale.name(), stale.target())])
            self.assertEqual(repo.delete_local_branches(["refs/heads/topic", "refs/heads/missing"]), [b"refs/heads/topic"])


if __name__ == "__main__":
    unittest.main()
