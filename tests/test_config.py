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


if __name__ == "__main__":
    unittest.main()
