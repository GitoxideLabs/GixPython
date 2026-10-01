"""Native submodule configuration, snapshots, and repository state."""

import concurrent.futures
import gc
from pathlib import Path
import tempfile
import unittest

import gix


OPTIONS = ["user.name=Submodule Tester", "user.email=submodule@example.invalid", "init.defaultBranch=main"]
MODULES = (b'[submodule "deps"]\npath = deps\nurl = ../source\n'
           b'branch = topic\nupdate = checkout\nignore = untracked\n'
           b'fetchRecurseSubmodules = on-demand\nshallow = true\n')


def fixture(path, modules=MODULES, *, object_hash=None, overrides=()):
    repo = gix.init(path, object_hash=object_hash, options=gix.OpenOptions.isolated().config_overrides(OPTIONS + list(overrides)))
    signature = gix.Signature("Submodule Tester", "submodule@example.invalid", 1700000000)
    child = repo.new_commit_as(signature, signature, "child", repo.empty_tree())
    blob = repo.write_blob(modules)
    tree = repo.write_object("tree", b"100644 .gitmodules\0" + bytes(blob) + b"160000 deps\0" + bytes(child.id))
    repo.commit_as(signature, signature, "HEAD", "root", tree)
    (path / ".gitmodules").write_bytes(modules)
    return repo, tree, child.id


@unittest.skipUnless("attributes" in gix.build_features(), "submodule support disabled")
class SubmoduleTests(unittest.TestCase):
    def test_raw_names_and_all_fields_preserve_bytes(self):
        name = b"raw-\xff"
        modules = gix.ModulesFile.from_bytes(MODULES.replace(b'"deps"', b'"' + name + b'"'), "config-path")
        self.assertEqual(list(modules.names()), [name])
        self.assertEqual(Path(modules.config_path()), Path("config-path"))
        self.assertEqual(modules.path(name), b"deps")
        self.assertEqual(modules.name_by_path(b"deps"), name)
        self.assertIsNone(modules.name_by_path(b"missing"))
        self.assertEqual(modules.url(name), b"../source")
        self.assertEqual(modules.branch(name), gix.SubmoduleBranch.Name("topic"))
        self.assertEqual(modules.branch(name).name_bytes(), b"topic")
        self.assertEqual(modules.update(name), gix.SubmoduleUpdate.Checkout)
        self.assertEqual(modules.ignore(name), gix.SubmoduleIgnore.Untracked)
        self.assertEqual(modules.fetch_recurse(name), gix.SubmoduleFetchRecurse.OnDemand)
        self.assertTrue(modules.shallow(name))
        self.assertIn(name, modules.config().to_bstring())
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo, _, child = fixture(root / "main", MODULES.replace(b'"deps"', b'"' + name + b'"'))
            with (Path(repo.git_dir()) / "config").open("ab") as config:
                config.write(b'\n[submodule "' + name + b'"]\nactive = true\n')
            repo = gix.open_opts(root / "main", gix.OpenOptions.isolated())
            with repo.submodules() as items:
                submodule = next(items)
                self.assertEqual(submodule.name(), name)
                self.assertEqual(submodule.validated_name(), name)
                self.assertEqual(submodule.index_id(), child)
                self.assertEqual(submodule.head_id(), child)
                self.assertTrue(submodule.is_active())

    def test_absence_and_head_and_index_fallback(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            empty = gix.init(root / "empty", options=gix.OpenOptions.isolated())
            self.assertIsNone(empty.open_modules_file())
            self.assertIsNone(empty.modules())
            self.assertIsNone(empty.submodules())
            for kind in gix.HashKind.all():
                with self.subTest(hash=str(kind)):
                    repo, tree, child = fixture(root / str(kind), object_hash=kind)
                    path = root / str(kind) / ".gitmodules"
                    path.unlink()
                    self.assertIsNone(repo.open_modules_file())
                    self.assertEqual(list(repo.modules().names()), [b"deps"])
                    with repo.submodules() as items:
                        submodule = next(items)
                    self.assertEqual(submodule.index_id(), child)
                    self.assertEqual(submodule.head_id(), child)
                    repo.index_from_tree(tree).write()
                    self.assertEqual(list(repo.modules().names()), [b"deps"])
                    replacement = repo.write_blob(MODULES.replace(b'"deps"', b'"index-only"'))
                    index = repo.index()
                    index.entry_mut_by_path_and_stage(".gitmodules", 0).id = replacement
                    index.write()
                    self.assertEqual(list(repo.modules().names()), [b"index-only"])

    def test_objects_retain_configuration_after_iterator_close_and_drop(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo, _, child = fixture(root / "main")
            iterator = repo.submodules()
            submodule = next(iterator)
            iterator.close()
            self.assertEqual(list(iterator), [])
            del iterator
            gc.collect()
            (root / "main" / ".gitmodules").write_bytes(MODULES.replace(b"../source", b"../replacement-longer"))
            self.assertEqual(submodule.url(), b"../source")
            self.assertEqual(submodule.path(), b"deps")
            self.assertEqual(submodule.index_id(), child)
            self.assertEqual(submodule.branch(), gix.SubmoduleBranch.Name("topic"))
            self.assertEqual(submodule.update(), gix.SubmoduleUpdate.Checkout)
            self.assertEqual(submodule.ignore(), gix.SubmoduleIgnore.Untracked)
            self.assertEqual(submodule.fetch_recurse(), gix.SubmoduleFetchRecurse.OnDemand)
            self.assertTrue(submodule.shallow())
            with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
                self.assertEqual([pool.submit(submodule.head_id).result() for _ in range(24)], [child] * 24)
            fresh = repo.open_modules_file()
            self.assertEqual(fresh.url("deps"), b"../replacement-longer")

    def test_field_validation_is_deferred_and_does_not_poison_other_modules(self):
        data = MODULES + b'[submodule "broken"]\npath = ../escape\nurl = ../source\nignore = nonsense\n'
        with tempfile.TemporaryDirectory() as directory:
            repo, _, _ = fixture(Path(directory) / "main", data)
            with repo.submodules() as items:
                good = next(items)
                bad = next(items)
                self.assertEqual(good.path(), b"deps")
                self.assertEqual(bad.name(), b"broken")
                with self.assertRaises(gix.Error):
                    bad.path()
                with self.assertRaises(gix.Error):
                    bad.ignore()
                self.assertEqual(good.url(), b"../source")
                self.assertEqual(list(items), [])
            unsafe = gix.ModulesFile.from_bytes(b'[submodule "deps"]\npath=deps\nupdate=!false\n')
            with self.assertRaises(gix.Error):
                unsafe.update("deps")

    def test_repository_overrides_and_explicit_configuration_snapshots(self):
        with tempfile.TemporaryDirectory() as directory:
            repo, _, _ = fixture(Path(directory) / "main", overrides=[
                "submodule.deps.url=../override", "submodule.deps.update=!echo target", "submodule.deps.branch=.",
                "submodule.deps.ignore=all", "submodule.deps.fetchRecurseSubmodules=false", "submodule.deps.active=true"
            ])
            config = repo.config_snapshot().plumbing()
            modules = gix.ModulesFile.from_bytes(MODULES, config=config)
            self.assertEqual(modules.url("deps"), b"../override")
            self.assertEqual(modules.update("deps"), gix.SubmoduleUpdate.Command("echo target"))
            self.assertEqual(modules.update("deps").command_bytes(), b"echo target")
            self.assertEqual(modules.branch("deps"), gix.SubmoduleBranch.CurrentInSuperproject)
            self.assertIsNone(modules.branch("deps").name_bytes())
            self.assertEqual(modules.ignore("deps"), gix.SubmoduleIgnore.All)
            self.assertEqual(modules.fetch_recurse("deps"), gix.SubmoduleFetchRecurse.Never)
            original = gix.ModulesFile.from_bytes(MODULES)
            self.assertIs(original.append_submodule_overrides(config), original)
            self.assertEqual(original.url("deps"), b"../override")
            with repo.submodules() as items:
                submodule = next(items)
            self.assertEqual(submodule.url(), b"../override")
            self.assertTrue(submodule.is_active())
            self.assertTrue(submodule.state().superproject_configuration)

    def test_missing_initialized_and_malformed_submodule_repositories(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo, _, child = fixture(root / "main")
            submodule = next(repo.submodules())
            state = submodule.state()
            self.assertFalse(state.repository_exists)
            self.assertFalse(state.is_old_form)
            self.assertFalse(state.worktree_checkout)
            self.assertFalse(state.superproject_configuration)
            self.assertIsNone(submodule.open())
            self.assertEqual(Path(submodule.git_dir()), root / "main" / ".git" / "modules" / "deps")
            self.assertEqual(Path(submodule.work_dir()), root / "main" / "deps")
            if "status" in gix.build_features():
                status = submodule.status(gix.SubmoduleIgnore.None_, False)
                self.assertIsNone(status.is_dirty())
                self.assertEqual(status.index_id, child)
            nested = gix.init(root / "main" / "deps", options=gix.OpenOptions.isolated())
            self.assertTrue(submodule.state().is_old_form)
            self.assertTrue(submodule.state().worktree_checkout)
            self.assertTrue(submodule.state().repository_exists)
            self.assertEqual(submodule.open().git_dir(), nested.git_dir())
            self.assertEqual(Path(submodule.git_dir_try_old_form()), Path(nested.git_dir()))
            (Path(nested.git_dir()) / "config").write_bytes(b"[invalid")
            with self.assertRaises(gix.Error):
                submodule.open()


if __name__ == "__main__":
    unittest.main()
