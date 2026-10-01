"""Native clone/fetch against disposable repositories and loopback servers only."""

import concurrent.futures
import _thread
import contextlib
import gc
import http.server
import os
from pathlib import Path
import subprocess
import ssl
import tempfile
import threading
import time
import unittest

import gix


@unittest.skipUnless("network" in gix.build_features(), "network feature disabled")
class NetworkTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="gix-network-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
        self.env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull,
                        GIT_TERMINAL_PROMPT="0", GIT_AUTHOR_NAME="Fixture", GIT_AUTHOR_EMAIL="fixture@example.invalid",
                        GIT_COMMITTER_NAME="Fixture", GIT_COMMITTER_EMAIL="fixture@example.invalid")
        self.source = self.root / "source"
        self.git("init", "--initial-branch=main", str(self.source))
        (self.source / "hello").write_bytes(b"hello from the native transport\n")
        self.git("-C", str(self.source), "add", "hello")
        self.git("-C", str(self.source), "commit", "-m", "initial")
        self.tip = self.git("-C", str(self.source), "rev-parse", "HEAD").strip().decode()
        self.options = gix.OpenOptions.isolated().config_overrides([
            "user.name=Fixture", "user.email=fixture@example.invalid", "credential.interactive=false",
        ])

    def git(self, *args, input=None):
        return subprocess.run(["git", *args], cwd=self.root, env=self.env, input=input,
                              check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout

    def test_prepare_is_immediate_and_abandonment_cleans_up(self):
        target = self.root / "abandoned"
        clone = gix.prepare_clone_bare(str(self.source), target, options=self.options)
        self.assertTrue((target / "HEAD").is_file())
        del clone
        gc.collect()
        for _ in range(100):
            if not target.exists():
                break
            time.sleep(0.01)
        self.assertFalse(target.exists())

    def test_clone_bare_and_consumption(self):
        progress = gix.Progress()
        clone = gix.prepare_clone_bare(str(self.source), self.root / "bare", options=self.options)
        repo, outcome = clone.fetch_only(progress=progress)
        self.assertTrue(repo.is_bare())
        self.assertEqual(progress.state, "succeeded")
        self.assertEqual(outcome.status, "Change")
        self.assertEqual(self.git("--git-dir", str(repo.git_dir()), "rev-parse", "HEAD").strip().decode(), self.tip)
        self.assertTrue(outcome.ref_map.remote_refs)
        with self.assertRaises(RuntimeError):
            clone.fetch_only()
        with self.assertRaises(RuntimeError):
            clone.persist()

    def test_invalid_clone_revision_preserves_destination_and_builder(self):
        target = self.root / "invalid-revision"
        clone = gix.prepare_clone(str(self.source), target, options=self.options)
        sentinel = target / "user-file"
        sentinel.write_bytes(b"preserve user data\n")
        for revision in ("not-a-full-ref", "", ":", "+HEAD", "HEAD:refs/heads/main", "refs/heads/*"):
            with self.subTest(revision=revision):
                with self.assertRaises(gix.Error):
                    clone.with_revision(revision)
                self.assertEqual(sentinel.read_bytes(), b"preserve user data\n")
        clone.with_revision("HEAD").with_revision(None)
        repo = clone.persist()
        self.assertEqual(Path(repo.workdir()), target)
        self.assertEqual(sentinel.read_bytes(), b"preserve user data\n")

    @unittest.skipUnless("worktree-mutation" in gix.build_features(), "checkout feature disabled")
    def test_clone_checkout_and_persist(self):
        target = self.root / "checkout"
        clone = gix.prepare_clone(str(self.source), target, options=self.options)
        checkout, _ = clone.fetch_then_checkout()
        self.assertEqual(Path(checkout.repo().workdir()), target)
        repo, outcome = checkout.main_worktree()
        self.assertEqual((target / "hello").read_bytes(), (self.source / "hello").read_bytes())
        self.assertEqual(outcome.files_updated, 1)
        self.assertEqual(outcome.errors, [])
        self.assertEqual(Path(repo.workdir()), target)
        with self.assertRaises(RuntimeError):
            checkout.main_worktree()

    def test_cancelled_fetch_can_be_retried(self):
        token = gix.CancellationToken()
        token.cancel()
        clone = gix.prepare_clone_bare(str(self.source), self.root / "cancelled", options=self.options)
        with self.assertRaises(gix.CancelledError):
            clone.fetch_only(cancel=token)
        repo, _ = clone.fetch_only()
        self.assertTrue(repo.is_bare())

    def test_remote_handshake_fetch_and_dry_run(self):
        repo = gix.init_bare(self.root / "fetch", options=self.options)
        remote = repo.remote_at(str(self.source)).with_refspecs([
            gix.RefSpec("+refs/heads/*:refs/remotes/origin/*", gix.Direction.Fetch),
        ], gix.Direction.Fetch)
        self.assertEqual(remote.url(gix.Direction.Fetch), os.fsencode(self.source))
        prepared = remote.connect(gix.Direction.Fetch).prepare_fetch()
        self.assertTrue(prepared.ref_map().remote_refs)
        result = prepared.with_dry_run(True).receive()
        self.assertEqual(result.status, "NoPackReceived")
        del prepared
        gc.collect()
        # A separate native remote can operate on the same repository independently.
        remote = repo.remote_at(str(self.source)).with_refspecs([
            gix.RefSpec("+refs/heads/*:refs/remotes/origin/*", gix.Direction.Fetch),
        ], gix.Direction.Fetch)
        result = remote.connect(gix.Direction.Fetch).prepare_fetch().receive()
        self.assertEqual(result.status, "Change")
        self.assertEqual(self.git("--git-dir", str(repo.git_dir()), "rev-parse", "refs/remotes/origin/main").strip().decode(), self.tip)

    def test_independent_connections_are_parallel(self):
        repo = gix.init_bare(self.root / "parallel", options=self.options)
        def refs(_):
            remote = repo.remote_at(str(self.source)).with_refspecs([
                gix.RefSpec("HEAD", gix.Direction.Fetch),
            ], gix.Direction.Fetch)
            mapping, handshake = remote.connect(gix.Direction.Fetch).ref_map()
            return len(mapping.remote_refs), handshake.server_protocol_version
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as executor:
            results = list(executor.map(refs, range(8)))
        self.assertTrue(all(count >= 1 for count, _ in results))

    def test_clone_reference_and_remote_configuration(self):
        self.git("-C", str(self.source), "branch", "topic")
        clone = gix.prepare_clone_bare(str(self.source), self.root / "named", options=self.options)
        clone.with_remote_name("upstream").with_ref_name("topic")
        repo, _ = clone.fetch_only()
        self.assertEqual(repo.remote_names(), [b"upstream"])
        self.assertEqual(repo.remote_default_name(gix.Direction.Fetch), b"upstream")
        self.assertEqual(repo.find_remote("upstream").name(), b"upstream")
        self.assertIsNone(repo.try_find_remote_without_url_rewrite("absent"))
        self.assertEqual(self.git("--git-dir", str(repo.git_dir()), "symbolic-ref", "HEAD").strip(), b"refs/heads/topic")
        clone = gix.prepare_clone_bare(str(self.source), self.root / "revision", options=self.options)
        repo, _ = clone.with_revision(self.tip).fetch_only()
        self.assertEqual((Path(repo.git_dir()) / "HEAD").read_text().strip(), self.tip)

    @unittest.skipUnless("sha256" in gix.build_features(), "SHA-256 feature disabled")
    def test_sha256_clone(self):
        source = self.root / "sha256-source"
        self.git("init", "--object-format=sha256", "--initial-branch=main", str(source))
        (source / "data").write_bytes(b"sha256 repository")
        self.git("-C", str(source), "add", "data")
        self.git("-C", str(source), "commit", "-m", "sha256")
        clone = gix.prepare_clone_bare(str(source), self.root / "sha256-clone", options=self.options)
        repo, result = clone.fetch_only()
        self.assertEqual(repo.object_hash(), gix.HashKind.SHA256)
        self.assertEqual(result.ref_map.object_hash, gix.HashKind.SHA256)

    @contextlib.contextmanager
    def server(self, *, tls=False, redirect=None, seen=None, entered=None, release=None, tls_maximum=None):
        source, env = self.source, self.env
        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"
            def log_message(self, *_):
                pass
            def do_GET(self):
                if seen is not None:
                    seen.append((self.path, dict(self.headers)))
                if entered is not None:
                    entered.set()
                if release is not None and not release.wait(timeout=10):
                    self.send_error(504)
                    return
                if redirect is not None and self.path.startswith("/redirect"):
                    self.send_response(302)
                    self.send_header("Location", redirect + self.path[len("/redirect"):])
                    self.send_header("Content-Length", "0")
                    self.send_header("Connection", "close")
                    self.end_headers()
                    return
                body = subprocess.run(["git", "upload-pack", "--stateless-rpc", "--advertise-refs", str(source)],
                                      env=env, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout
                self.respond(b"001e# service=git-upload-pack\n0000" + body, "application/x-git-upload-pack-advertisement")
            def do_POST(self):
                request = self.rfile.read(int(self.headers["Content-Length"]))
                body = subprocess.run(["git", "upload-pack", "--stateless-rpc", str(source)], input=request,
                                      env=env, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout
                self.respond(body, "application/x-git-upload-pack-result")
            def respond(self, body, content_type):
                self.send_response(200)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(body)))
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(body)
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        if tls:
            fixture = Path(__file__).parent / "fixtures" / "tls"
            context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            if tls_maximum is not None:
                context.maximum_version = tls_maximum
            context.load_cert_chain(fixture / "server.pem", fixture / "server-key.pem")
            server.socket = context.wrap_socket(server.socket, server_side=True)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            yield f"{'https' if tls else 'http'}://{'localhost' if tls else '127.0.0.1'}:{server.server_port}/repo"
        finally:
            server.shutdown()
            server.server_close()
            worker.join(timeout=5)

    @unittest.skipUnless("http" in gix.build_features(), "HTTP feature disabled")
    def test_clone_from_loopback_http(self):
        with self.server() as url:
            clone = gix.prepare_clone_bare(url, self.root / "http", options=self.options)
            repo, outcome = clone.fetch_only()
        self.assertEqual(outcome.status, "Change")
        self.assertEqual(self.git("--git-dir", str(repo.git_dir()), "rev-parse", "HEAD").strip().decode(), self.tip)

    @unittest.skipUnless("http" in gix.build_features(), "HTTP feature disabled")
    def test_configured_http2_does_not_silently_downgrade(self):
        with self.server() as url:
            options = self.options.config_overrides(["http.version=HTTP/2"])
            clone = gix.prepare_clone_bare(url, self.root / "http2", options=options)
            with self.assertRaises(gix.Error):
                clone.fetch_only()

    @unittest.skipUnless("https" in gix.build_features(), "HTTPS feature disabled")
    def test_tls_verification_custom_ca_and_hostname(self):
        ca = Path(__file__).parent / "fixtures" / "tls" / "ca.pem"
        trusted = self.options.config_overrides([f"http.sslCAInfo={ca.resolve()}"])
        with self.server(tls=True) as url:
            clone = gix.prepare_clone_bare(url, self.root / "untrusted", options=self.options)
            with self.assertRaises(gix.Error):
                clone.fetch_only()
            clone = gix.prepare_clone_bare(url, self.root / "trusted", options=trusted)
            repo, _ = clone.fetch_only()
            self.assertTrue(repo.is_bare())
            mismatched = url.replace("localhost", "127.0.0.1")
            clone = gix.prepare_clone_bare(mismatched, self.root / "mismatched", options=trusted)
            with self.assertRaises(gix.Error):
                clone.fetch_only()
            disabled = self.options.config_overrides(["http.sslVerify=false"])
            clone = gix.prepare_clone_bare(mismatched, self.root / "explicit-no-verify", options=disabled)
            repo, _ = clone.fetch_only()
            self.assertTrue(repo.is_bare())

    @unittest.skipUnless("https" in gix.build_features(), "HTTPS feature disabled")
    def test_configured_tls_version_is_enforced(self):
        ca = Path(__file__).parent / "fixtures" / "tls" / "ca.pem"
        with self.server(tls=True, tls_maximum=ssl.TLSVersion.TLSv1_2) as url:
            options = self.options.config_overrides([f"http.sslCAInfo={ca.resolve()}", "http.sslVersion=tlsv1.3"])
            clone = gix.prepare_clone_bare(url, self.root / "wrong-version", options=options)
            with self.assertRaises(gix.Error):
                clone.fetch_only()
            options = self.options.config_overrides([f"http.sslCAInfo={ca.resolve()}", "http.sslVersion=tlsv1.2"])
            clone = gix.prepare_clone_bare(url, self.root / "tls12", options=options)
            repo, _ = clone.fetch_only()
            self.assertTrue(repo.is_bare())

    @unittest.skipUnless("http" in gix.build_features(), "HTTP feature disabled")
    def test_redirect_does_not_forward_private_headers(self):
        seen = []
        with self.server(seen=seen) as destination:
            with self.server(redirect=destination) as source:
                url = source.replace("/repo", "/redirect")
                options = self.options.config_overrides(["http.extraHeader=private-token: secret"])
                clone = gix.prepare_clone_bare(url, self.root / "private-header", options=options)
                with self.assertRaises(gix.Error):
                    clone.fetch_only()
                self.assertEqual(seen, [])
                clone = gix.prepare_clone_bare(url, self.root / "ordinary-redirect", options=self.options)
                repo, _ = clone.fetch_only()
                self.assertTrue(repo.is_bare())
                self.assertTrue(seen)

    @unittest.skipUnless("https" in gix.build_features(), "HTTPS feature disabled")
    def test_tls_redirect_cannot_downgrade_to_plain_http(self):
        seen = []
        ca = Path(__file__).parent / "fixtures" / "tls" / "ca.pem"
        options = self.options.config_overrides([f"http.sslCAInfo={ca.resolve()}"])
        with self.server(seen=seen) as destination:
            with self.server(tls=True, redirect=destination) as source:
                clone = gix.prepare_clone_bare(source.replace("/repo", "/redirect"), self.root / "downgrade", options=options)
                with self.assertRaises(gix.Error):
                    clone.fetch_only()
                self.assertEqual(seen, [])

    @unittest.skipUnless("http" in gix.build_features(), "HTTP feature disabled")
    def test_progress_and_cancellation_while_native_io_is_pending(self):
        entered, release = threading.Event(), threading.Event()
        progress, token = gix.Progress(), gix.CancellationToken()
        with self.server(entered=entered, release=release) as url:
            clone = gix.prepare_clone_bare(url, self.root / "pending", options=self.options)
            with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
                future = executor.submit(clone.fetch_only, progress=progress, cancel=token)
                try:
                    self.assertTrue(entered.wait(timeout=5))
                    self.assertEqual(progress.snapshot()["state"], "running")
                    token.cancel()
                    # Native blocking I/O finishes cooperatively; don't leave the test server stalled.
                    time.sleep(0.1)
                finally:
                    release.set()
                with self.assertRaises(gix.CancelledError):
                    future.result(timeout=5)
                self.assertEqual(progress.state, "cancelled")

    @unittest.skipUnless("http" in gix.build_features(), "HTTP feature disabled")
    def test_keyboard_interrupt_is_propagated_while_native_io_is_pending(self):
        entered, release = threading.Event(), threading.Event()
        progress = gix.Progress()
        with self.server(entered=entered, release=release) as url:
            clone = gix.prepare_clone_bare(url, self.root / "interrupted", options=self.options)
            def interrupt():
                if entered.wait(timeout=5):
                    _thread.interrupt_main()
                    time.sleep(0.15)
                release.set()
            sender = threading.Thread(target=interrupt, daemon=True)
            sender.start()
            try:
                with self.assertRaises(KeyboardInterrupt):
                    clone.fetch_only(progress=progress)
            finally:
                release.set()
                sender.join(timeout=5)
            self.assertEqual(progress.state, "cancelled")


if __name__ == "__main__":
    unittest.main()
