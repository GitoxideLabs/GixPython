"""Native history metadata with isolated repositories and local verifier helpers."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import gix


class HistoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="gix-history-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.path = self.root / "repo"
        self.options = gix.OpenOptions.isolated().config_overrides([
            "user.name=Fixture", "user.email=old@example.invalid", "commit.gpgSign=false",
        ])
        self.repo = gix.init(self.path, options=self.options)
        signature = gix.Signature("Fixture", "old@example.invalid", 1700000000)
        tree = self.repo.empty_tree()
        self.first = self.repo.commit_as(signature, signature, "HEAD", "first", tree)
        self.tip = self.repo.commit_as(signature, signature, "HEAD", "tip", tree, [self.first])
        self.env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
        self.env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull, GIT_TERMINAL_PROMPT="0")

    def git(self, *args):
        return subprocess.run(["git", "-C", str(self.path), *args], env=self.env, check=True,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout

    def test_commit_graph_is_optional_and_bounds_are_checked(self):
        self.assertIsNone(self.repo.commit_graph_if_enabled())
        with self.assertRaises(gix.Error):
            self.repo.commit_graph()
        self.git("commit-graph", "write", "--reachable")
        graph = self.repo.commit_graph()
        self.assertEqual(graph.num_commits(), 2)
        self.assertEqual(set(map(str, graph.iter_ids())), {str(self.first), str(self.tip)})
        position = graph.lookup(self.tip)
        commit = graph.commit_at(position)
        self.assertEqual(commit.id(), self.tip)
        self.assertEqual(commit.generation(), 2)
        self.assertEqual(commit.root_tree_id(), self.repo.empty_tree().id)
        self.assertEqual([graph.id_at(p) for p in commit.iter_parents()], [self.first])
        self.assertEqual(graph.commit_by_id(self.first).generation(), 1)
        with self.assertRaises(IndexError):
            graph.commit_at(graph.num_commits())
        with self.assertRaises(IndexError):
            graph.id_at(2**32 - 1)
        self.assertEqual(len(list(graph.iter_commits())), 2)
        self.assertEqual(graph.object_hash(), self.repo.object_hash())

    @unittest.skipUnless("revision" in gix.build_features(), "revision feature disabled")
    def test_reusable_revision_graph_preserves_native_cache(self):
        graph = self.repo.revision_graph()
        self.assertTrue(graph.is_empty())
        self.assertEqual(self.repo.merge_base_with_graph("HEAD", "HEAD~1", graph), self.first)
        self.assertGreater(graph.len(), 0)
        self.assertTrue(graph.contains(self.tip))
        self.assertEqual(self.repo.merge_bases_many_with_graph(self.first, [self.tip], graph), [self.first])
        self.assertEqual(self.repo.merge_base_octopus_with_graph([self.first, self.tip], graph), self.first)
        graph.clear()
        self.assertEqual(graph.len(), 0)
        graph.close()
        with self.assertRaises(RuntimeError):
            graph.len()

    @unittest.skipUnless("revision" in gix.build_features(), "revision feature disabled")
    def test_describe_matches_git(self):
        self.git("tag", "v1", str(self.first))
        platform = self.repo.find_commit(self.tip).describe().names(gix.SelectRef.AllTags)
        self.assertEqual(str(platform.try_format()), self.git("describe", "--tags", str(self.tip)).strip().decode())
        resolution = platform.try_resolve()
        self.assertEqual(resolution.name, b"v1")
        self.assertEqual(resolution.depth, 1)
        self.assertEqual(resolution.id, self.tip)
        self.assertFalse(resolution.format().is_exact_match())
        self.assertIsNone(self.repo.find_commit(self.tip).describe().try_resolve())
        fallback = self.repo.find_commit(self.tip).describe()
        self.assertTrue(str(self.tip).startswith(str(fallback.format())))
        self.assertEqual(fallback.try_resolve().id, self.tip)
        self.git("commit-graph", "write", "--reachable")
        self.assertEqual(platform.try_resolve_with_cache(self.repo.commit_graph()).name, b"v1")

    @unittest.skipUnless("mailmap" in gix.build_features(), "mailmap feature disabled")
    def test_mailmap_resolution_and_snapshot_iterator(self):
        (self.path / ".mailmap").write_bytes(b"Canonical <canonical@example.invalid> <old@example.invalid>\n")
        snapshot = self.repo.open_mailmap()
        signature = self.repo.find_commit(self.tip).author()
        resolved = snapshot.resolve(signature)
        self.assertEqual((resolved.name, resolved.email), (b"Canonical", b"canonical@example.invalid"))
        self.assertEqual(resolved.time.seconds, signature.time.seconds)
        entries = snapshot.iter()
        (self.path / ".mailmap").write_bytes(b"Changed <changed@example.invalid> <old@example.invalid>\n")
        self.repo.open_mailmap_into(snapshot)
        self.assertEqual(next(entries).new_name, b"Canonical")
        self.assertEqual(snapshot.resolve(signature).name, b"Changed")
        self.assertIsNone(snapshot.try_resolve(gix.Signature("Unknown", "unknown@example.invalid", 0)))
        self.assertEqual(snapshot.entries()[0].old_email, b"old@example.invalid")
        entries.close()

    @unittest.skipUnless("signing" in gix.build_features(), "signing feature disabled")
    def test_unsigned_commit_and_disabled_signing_options(self):
        commit = self.repo.find_commit(self.tip)
        self.assertIsNone(commit.signature())
        self.assertIsNone(commit.verify_signature())
        self.assertIsNone(self.repo.commit_signing_options_if_enabled())
        self.assertEqual(self.repo.commit_signing_options().format, "OpenPgp")

    @unittest.skipUnless("signing" in gix.build_features() and shutil.which("ssh-keygen"), "SSH signing helper unavailable")
    def test_native_ssh_signing_verification_and_bad_signature(self):
        key = self.root / "signing-key"
        subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(key)], env=self.env,
                       check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        allowed = self.root / "allowed-signers"
        allowed.write_bytes(b"fixture@example.invalid " + key.with_suffix(".pub").read_bytes())
        options = self.options.config_overrides([
            "gpg.format=ssh", f"user.signingKey={key}", f"gpg.ssh.allowedSignersFile={allowed}",
        ])
        repo = gix.open_opts(self.path, options)
        signed = repo.find_commit(self.tip).signed()
        self.assertNotEqual(signed.id, self.tip)
        self.assertEqual(repo.head_id(), self.tip)
        signature, data = signed.signature()
        self.assertTrue(signature.startswith(b"-----BEGIN SSH SIGNATURE-----"))
        self.assertNotIn(b"gpgsig", data.to_bstring())
        result = signed.verify_signature()
        self.assertTrue(result.is_valid())
        self.assertEqual((result.format, result.status, result.signer), ("Ssh", "Good", b"fixture@example.invalid"))
        bad = repo.find_commit(repo.write_object("commit", signed.data.replace(b"\n\ntip", b"\n\ntampered")))
        self.assertFalse(bad.verify_signature().is_valid())
