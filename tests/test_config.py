import gc
from pathlib import Path
import tempfile
import unittest

import gix


class ConfigurationTests(unittest.TestCase):
    def test_snapshot_autocommit_forget_and_rollback(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = gix.init(directory, options=gix.OpenOptions.isolated())
            initial = repo.config_snapshot()
            with repo.config_snapshot_mut() as edit:
                edit.append_config([b"user.name=Example", b"user.email=example@example.com"])
                with self.assertRaises(RuntimeError):
                    repo.reload()
                self.assertIsNone(repo.config_snapshot().string("user.name"))
            self.assertIsNone(initial.string("user.name"))
            self.assertEqual(repo.author().name, b"Example")
            try:
                with repo.config_snapshot_mut() as edit:
                    edit.set_raw_value("user.name", b"Committed on unwind")
                    raise LookupError("original")
            except LookupError:
                pass
            self.assertEqual(repo.author().name, b"Committed on unwind")
            edit = repo.config_snapshot_mut()
            edit.set_raw_value("user.name", b"forgotten")
            self.assertEqual(edit.forget().string("user.name"), b"forgotten")
            self.assertEqual(repo.author().name, b"Committed on unwind")
            edit = repo.config_snapshot_mut()
            edit.set_raw_value("user.name", b"temporary")
            with edit.commit_auto_rollback() as rollback:
                self.assertEqual(rollback.repo.author().name, b"temporary")
                with self.assertRaises(RuntimeError):
                    repo.set_workdir(None)
            self.assertEqual(repo.author().name, b"Committed on unwind")
            edit = repo.config_snapshot_mut()
            edit.set_raw_value("user.name", b"destructor")
            del edit
            gc.collect()
            self.assertEqual(repo.author().name, b"destructor")
            repo.reload()
            self.assertIsNone(repo.author())

    def test_file_transaction_requires_commit_and_reload(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = gix.init(directory, options=gix.OpenOptions.isolated())
            path = Path(repo.config_path("local"))
            before = path.read_bytes()
            with repo.config_file_mut(path) as edit:
                edit.set_raw_value("user.name", b"discard me")
            self.assertEqual(path.read_bytes(), before)
            with repo.config_file_mut(path) as edit:
                edit.set_raw_value("user.name", b"saved")
                edit.commit()
            self.assertIsNone(repo.config_snapshot().string("user.name"))
            repo.reload()
            self.assertEqual(repo.config_snapshot().string("user.name"), b"saved")
            self.assertFalse(Path(str(path) + ".lock").exists())

    @unittest.skipUnless("revision" in gix.build_features(), "revision feature disabled")
    def test_memory_config_destructors_survive_an_active_native_owner(self):
        for rollback, cancel in ((False, False), (False, True), (True, False), (True, True)):
            with self.subTest(rollback=rollback, cancel=cancel), tempfile.TemporaryDirectory() as directory:
                options = gix.OpenOptions.isolated().config_overrides([
                    "user.name=original", "user.email=fixture@example.invalid"
                ])
                repo = gix.init(directory, options=options)
                commit = repo.commit("HEAD", "initial", repo.empty_tree())
                memory = repo.with_object_memory()
                transaction = memory.config_snapshot_mut()
                transaction.set_raw_value("user.name", "temporary")
                if rollback:
                    transaction = transaction.commit_auto_rollback()
                token = gix.CancellationToken()
                walk = memory.rev_walk([commit]).all(cancel=token)
                self.addCleanup(walk.close)
                next(walk)
                del transaction
                gc.collect()
                if cancel:
                    token.cancel()
                    with self.assertRaises(gix.CancelledError):
                        next(walk)
                else:
                    walk.close()
                self.assertEqual(memory.author().name, b"original" if rollback else b"temporary")
                # A queued destructor must also release its mutation lease.
                with memory.config_snapshot_mut() as edit:
                    edit.set_raw_value("user.name", "next edit")
                self.assertEqual(memory.author().name, b"next edit")

    @unittest.skipUnless("revision" in gix.build_features(), "revision feature disabled")
    def test_memory_explicit_config_updates_can_retry_after_overlap(self):
        for operation in ("commit", "commit_auto_rollback", "rollback"):
            with self.subTest(operation=operation), tempfile.TemporaryDirectory() as directory:
                options = gix.OpenOptions.isolated().config_overrides([
                    "user.name=original", "user.email=fixture@example.invalid"
                ])
                repo = gix.init(directory, options=options)
                commit = repo.commit("HEAD", "initial", repo.empty_tree())
                memory = repo.with_object_memory()
                transaction = memory.config_snapshot_mut()
                transaction.set_raw_value("user.name", "temporary")
                rollback = operation == "rollback"
                if rollback:
                    transaction = transaction.commit_auto_rollback()
                walk = memory.rev_walk([commit]).all()
                self.addCleanup(walk.close)
                next(walk)
                apply = getattr(transaction, operation)
                with self.assertRaises(RuntimeError):
                    apply()
                walk.close()
                self.assertEqual(memory.author().name, b"temporary" if rollback else b"original")
                result = apply()
                self.assertEqual(memory.author().name, b"original" if rollback else b"temporary")
                if operation == "commit_auto_rollback":
                    result.rollback()
                    self.assertEqual(memory.author().name, b"original")


if __name__ == "__main__":
    unittest.main()
