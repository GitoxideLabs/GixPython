"""Create a disposable repository and use native repository operations.

Run with an installed GixPython or PYTHONPATH=python from the source checkout.
"""

from pathlib import Path
from tempfile import TemporaryDirectory

import gix


def main():
    with TemporaryDirectory() as directory:
        repo = gix.init(Path(directory) / "example", options=gix.OpenOptions.isolated().config_overrides([
            "user.name=Example", "user.email=example@example.invalid",
        ]))
        blob = repo.write_blob(b"hello from Rust\n")
        with repo.empty_tree().edit() as editor:
            editor.upsert(b"hello.txt", "blob", blob)
            tree = editor.write()
        first = repo.commit("HEAD", "Initial commit", tree, [])

        # A revision is accepted where the native method takes an object ID.
        commit = repo.find_commit("HEAD")
        assert commit.id == first
        assert commit.tree().find_entry(b"hello.txt").object().data == b"hello from Rust\n"
        print(commit.message_raw().decode("utf-8"))

        with repo.rev_walk(["HEAD"]).all() as history:
            assert next(history).id == first
            assert list(history) == []

        branch = repo.find_reference("HEAD")
        print(branch.name(), branch.target())
        assert repo.object_hash() in gix.HashKind.all()


if __name__ == "__main__":
    main()
