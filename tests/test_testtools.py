"""Exercise native fixture caches in disposable child-process workspaces."""

import os
from pathlib import Path
import subprocess
import shutil
import sys
import tempfile
import textwrap
import unittest

import gix


class TesttoolsTests(unittest.TestCase):
    def run_fixture_test(self, source, directory):
        env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_") and key != "GIX_TEST_FIXTURE_HASH"}
        env.update(PYTHONPATH=str(Path(gix.__file__).parent.parent), GIX_TEST_IGNORE_ARCHIVES="1")
        result = subprocess.run([sys.executable, "-c", textwrap.dedent(source)], cwd=directory,
                                env=env, capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_python_fixture_cache_postprocessing_and_writable_copy(self):
        with tempfile.TemporaryDirectory() as directory:
            self.run_fixture_test("""
                from pathlib import Path
                import concurrent.futures
                import gix
                from gix import testtools as t

                seen = []
                def create(state):
                    seen.append(state.is_uninitialized())
                    if state.is_uninitialized():
                        repo = gix.init(state.path() / "repo", options=gix.OpenOptions.isolated())
                        (state.path() / "blob-id").write_text(str(repo.write_blob(b"raw \\xff")))
                    return (state.path() / "blob-id").read_text()

                # Concurrent first callers share one initialized fixture.
                with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
                    fixtures = list(pool.map(lambda _: t.rust_fixture_read_only("cached", 1, create), range(4)))
                assert seen.count(True) == 1 and seen.count(False) == 3, seen
                assert len(set(fixtures)) == 1
                path, blob_id = fixtures[0]
                assert isinstance(path, Path)
                assert gix.open_opts(path / "repo", gix.OpenOptions.isolated()).find_blob(blob_id).data == b"raw \\xff"

                temporary, value = t.rust_fixture_writable("cached", 1, t.Creation.CopyFromReadOnly, create)
                with temporary as writable:
                    copy = writable.path()
                    assert value == blob_id and copy != path
                    (copy / "blob-id").write_text("changed")
                    assert (path / "blob-id").read_text() == blob_id
                assert not copy.exists()
                temporary.close()
                try:
                    temporary.path()
                except RuntimeError:
                    pass
                else:
                    raise AssertionError("closed fixture still exposes a path")

                _, versioned = t.rust_fixture_read_only("cached", 2, create)
                assert seen[-1] is True and versioned == blob_id
                # Execute initializes at the final location, including absolute paths.
                def absolute(state):
                    assert state.is_uninitialized()
                    return state.path()
                owned, final_path = t.rust_fixture_writable("absolute", 1, t.Creation.Execute, absolute)
                assert owned.path() == final_path
                del owned
                assert not final_path.exists()
            """, directory)
            # The generated fixture persists across processes and calls the closure as Fresh.
            self.run_fixture_test("""
                from gix import testtools as t
                def cached(state):
                    assert not state.is_uninitialized()
                    return (state.path() / "blob-id").read_text()
                path, value = t.rust_fixture_read_only("cached", 1, cached)
                assert value == (path / "blob-id").read_text()
            """, directory)

    def test_scripted_fixture_python_post_and_arguments(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixtures = root / "tests/fixtures"
            fixtures.mkdir(parents=True)
            script = fixtures / "make.sh"
            script.write_text('set -eu\ngit init repo\nprintf "%s" "$1" > value\n')
            self.run_fixture_test("""
                from gix import testtools as t
                import os
                # Fixture commands must clear inherited redirects to another repository.
                os.environ["GIT_DIR"] = str(t.fixture_path("unrelated-repository"))
                os.environ["GIT_WORK_TREE"] = str(t.fixture_path("unrelated-worktree"))
                states = []
                def post(state):
                    states.append(state.is_uninitialized())
                    if state.is_uninitialized():
                        (state.path() / "value").write_bytes((state.path() / "value").read_bytes() + b"-post")
                    return (state.path() / "value").read_bytes()

                path, value = t.scripted_fixture_read_only_with_args_with_post("make.sh", ["input with spaces"], 1, post)
                again, same = t.scripted_fixture_read_only_with_args_with_post("make.sh", ["input with spaces"], 1, post)
                assert path == again and value == same == b"input with spaces-post"
                assert states == [True, False], states
                assert t.git(path / "repo", "rev-parse --is-inside-work-tree").strip() == "true"
                with t.scripted_fixture_writable_with_args("make.sh", ["writable"], t.Creation.Execute) as writable:
                    assert (writable.path() / "value").read_bytes() == b"writable"
                assert t.fixture_bytes("make.sh").startswith(b"set -eu")
                assert not t.fixture_path("unrelated-repository").exists()
                writable, result = t.scripted_fixture_writable_with_args_with_post(
                    "make.sh", ["copy"], t.Creation.CopyFromReadOnly, 1, post
                )
                with writable:
                    assert result == b"copy-post"
                    assert (writable.path() / "value").read_bytes() == result
                t.fixture_path("plain.sh").write_text('printf original > value\\n')
                plain, result = t.scripted_fixture_read_only_with_post("plain.sh", 1, post)
                assert result == b"original-post"
                temporary, result = t.scripted_fixture_writable_with_post("plain.sh", 1, post)
                with temporary:
                    assert result == b"original-post" and temporary.path() != plain
                assert t.scripted_fixture_read_only("plain.sh").is_dir()
                assert t.scripted_fixture_read_only_with_args("make.sh", ["plain-args"]).is_dir()
            """, directory)

    @unittest.skipIf(sys.platform == "win32", "native archive generation is disabled on Windows")
    def test_archive_extraction_skips_initialization(self):
        with tempfile.TemporaryDirectory() as directory:
            self.run_fixture_test("""
                import os
                from pathlib import Path
                import gix
                from gix import testtools as t
                os.environ.pop("GIX_TEST_IGNORE_ARCHIVES", None)
                os.environ["GIX_TEST_CREATE_ARCHIVES_EVEN_ON_CI"] = "1"
                def create(state):
                    assert state.is_uninitialized()
                    (state.path() / "value").write_bytes(b"archived")
                    return "created"
                path, value = t.rust_fixture_read_only("archive", 1, create)
                assert value == "created"
                hash_name = "sha1" if gix.HashKind.SHA1 is not None else "sha256"
                assert Path(f"tests/fixtures/generated-archives/rust-archive_{hash_name}.tar").is_file()
            """, directory)
            shutil.rmtree(Path(directory) / "tests/fixtures/generated-do-not-edit")
            self.run_fixture_test("""
                import os
                from gix import testtools as t
                os.environ.pop("GIX_TEST_IGNORE_ARCHIVES", None)
                def cached(state):
                    assert not state.is_uninitialized()
                    return (state.path() / "value").read_bytes()
                assert t.rust_fixture_read_only("archive", 1, cached)[1] == b"archived"
            """, directory)

    def test_callback_exception_identity_and_failed_fixture_retry(self):
        with tempfile.TemporaryDirectory() as directory:
            self.run_fixture_test("""
                from gix import testtools as t
                class Failure(Exception):
                    pass
                failure = Failure("post failed")
                def fail(state):
                    (state.path() / "partial").write_text("incomplete")
                    raise failure
                try:
                    t.rust_fixture_read_only("failure", 1, fail)
                except Failure as caught:
                    assert caught is failure
                else:
                    raise AssertionError("callback exception was suppressed")
                def retry(state):
                    assert state.is_uninitialized()
                    assert not (state.path() / "partial").exists()
                    return "retried"
                assert t.rust_fixture_read_only("failure", 1, retry)[1] == "retried"
                for name in ("", ".", "..", "../escape", "/absolute"):
                    try:
                        t.rust_fixture_read_only(name, 1, retry)
                    except ValueError:
                        pass
                    else:
                        raise AssertionError(name)
                try:
                    t.rust_fixture_read_only("not-callable", 1, 42)
                except TypeError:
                    pass
                else:
                    raise AssertionError("non-callable accepted")
                try:
                    t.scripted_fixture_read_only("missing.sh")
                except Exception as error:
                    assert "missing.sh" in str(error)
                else:
                    raise AssertionError("missing script accepted")
            """, directory)

    @unittest.skipIf(sys.platform == "win32", "POSIX termination signals")
    def test_fixtures_preserve_python_signal_handlers(self):
        with tempfile.TemporaryDirectory() as directory:
            self.run_fixture_test("""
                import os
                import signal
                import gix
                received = []
                signal.signal(signal.SIGTERM, lambda signum, frame: received.append(signum))
                gix.testtools.rust_fixture_read_only("signals", 1, lambda state: None)
                os.kill(os.getpid(), signal.SIGTERM)
                assert received == [signal.SIGTERM]
            """, directory)


if __name__ == "__main__":
    unittest.main()
