import tempfile
import unittest

import gix


class BranchTests(unittest.TestCase):
    def test_configured_branch_names(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = gix.init(directory, options=gix.OpenOptions.isolated())
            self.assertEqual(repo.branch_names(), [])
            with repo.config_snapshot_mut() as config:
                config.set_raw_value("branch.main.remote", "origin")
                config.set_raw_value("branch.topic.remote", "origin")
            self.assertEqual(repo.branch_names(), ["main", "topic"])

    @unittest.skipUnless("network" in gix.build_features(), "network feature disabled")
    def test_branch_upstream_push_and_reverse_refspec_mappings(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = gix.init(directory, options=gix.OpenOptions.isolated())
            with repo.config_snapshot_mut() as config:
                config.append_config([
                    "remote.origin.url=https://example.invalid/repo",
                    "remote.origin.fetch=+refs/heads/*:refs/remotes/origin/*",
                    "branch.main.remote=origin",
                    "branch.main.merge=refs/heads/main",
                    "push.default=current",
                ])
            fetch = gix.Direction.Fetch
            push = gix.Direction.Push
            self.assertEqual(repo.branch_remote_name("main", fetch), b"origin")
            self.assertEqual(repo.branch_remote_ref_name("refs/heads/main", fetch), b"refs/heads/main")
            self.assertEqual(repo.branch_remote_tracking_ref_name("refs/heads/main", fetch), b"refs/remotes/origin/main")
            self.assertEqual(repo.branch_remote_ref_name("refs/heads/main", push), b"refs/heads/main")
            remote = repo.branch_remote("main", fetch)
            self.assertEqual(remote.name(), b"origin")
            upstream, remote = repo.upstream_branch_and_remote_for_tracking_branch("refs/remotes/origin/main")
            self.assertEqual(upstream, b"refs/heads/main")
            self.assertEqual(remote.name(), b"origin")
            self.assertIsNone(repo.branch_remote_name("unknown", fetch))
            self.assertIsNone(repo.upstream_branch_and_remote_for_tracking_branch("refs/remotes/missing/main"))
            with self.assertRaises(gix.Error):
                repo.upstream_branch_and_remote_for_tracking_branch("refs/heads/main")
            with repo.config_snapshot_mut() as config:
                config.append_config([
                    "remote.duplicate.url=https://example.invalid/duplicate",
                    "remote.duplicate.fetch=+refs/heads/*:refs/remotes/origin/*",
                ])
            with self.assertRaises(gix.Error):
                repo.upstream_branch_and_remote_for_tracking_branch("refs/remotes/origin/main")

    @unittest.skipUnless("network" in gix.build_features(), "network feature disabled")
    def test_reference_remote_shortcuts(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = gix.init(directory, options=gix.OpenOptions.isolated())
            with repo.config_snapshot_mut() as config:
                config.append_config([
                    "user.name=Test",
                    "user.email=test@example.com",
                    "remote.origin.url=https://example.invalid/repo",
                    "remote.origin.fetch=+refs/heads/*:refs/remotes/origin/*",
                    "branch.main.remote=origin",
                    "branch.main.merge=refs/heads/main",
                ])
            commit = repo.new_commit("initial", repo.empty_tree())
            branch = repo.reference("refs/heads/main", commit, gix.PreviousValue.Any, "test")
            self.assertEqual(branch.remote_name(gix.Direction.Fetch), b"origin")
            self.assertEqual(branch.remote_ref_name(gix.Direction.Fetch), b"refs/heads/main")
            self.assertEqual(branch.remote_tracking_ref_name(gix.Direction.Fetch), b"refs/remotes/origin/main")
            self.assertEqual(branch.remote(gix.Direction.Fetch).name(), b"origin")
            repo.edit_reference(gix.RefEdit.update("HEAD", gix.Target.Symbolic("refs/heads/main"), gix.PreviousValue.Any, "test"))
            self.assertEqual(repo.head().into_remote(gix.Direction.Fetch).name(), b"origin")


if __name__ == "__main__":
    unittest.main()
