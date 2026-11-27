"""Offline checks for the CI reference-boot input helper; no network or KVM."""
import base64
import hashlib
import io
import json
import os
import struct
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from guest import ci_inputs
from guest.lock import VERSIONS, file_hash, load_lock

COMMIT = "0123456789abcdef0123456789abcdef01234567"
CHART = b"synthetic retained chart"
FIXTURE = json.dumps({"files": {}, "chartChunks": [base64.b64encode(CHART[:9]).decode(), base64.b64encode(CHART[9:]).decode()]}).encode()


def row(use, name, version, path, data):
    return {"use": use, "name": name, "version": version, "path": path, "url": "https://example.test/" + path,
            "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}


def synthetic_sources():
    payloads = {"base/debian.qcow2": b"synthetic base", "debs/tini.deb": b"synthetic deb", "tools/kind": b"synthetic kind",
                "kmx/install.json": FIXTURE}
    rows = [row("package", "debian-base", VERSIONS["debianBase"], "base/debian.qcow2", payloads["base/debian.qcow2"]),
            row("package", "tini", "0.19.0-3+b8", "debs/tini.deb", payloads["debs/tini.deb"]),
            row("tool", "kind", VERSIONS["kind"], "tools/kind", payloads["tools/kind"]),
            row("chart-fixture", "kmx-install", VERSIONS["sourceOfflineProfile"]["kmxSourceCommit"], "kmx/install.json", FIXTURE)]
    return {"schemaVersion": "bridge-guest-ci-sources/v1", "architecture": "amd64", "files": rows}, payloads


class Server:
    def __init__(self, payloads, override=None):
        self.payloads, self.override, self.requests = payloads, override or {}, []

    def __call__(self, url, timeout):
        self.requests.append(url)
        path = url.removeprefix("https://example.test/")
        return io.BytesIO(self.override.get(path, self.payloads[path]))


class SourcesTests(unittest.TestCase):
    def test_checked_in_sources_match_the_version_policy(self):
        sources = ci_inputs.load_sources(ci_inputs.SOURCES)
        rows = {(r["use"], r["name"]): r for r in sources["files"]}
        self.assertEqual(VERSIONS["debianBase"], rows["package", "debian-base"]["version"])
        self.assertEqual(VERSIONS["kind"], rows["tool", "kind"]["version"])
        (fixture,) = [r for r in sources["files"] if r["use"] == "chart-fixture"]
        self.assertIn("/" + VERSIONS["sourceOfflineProfile"]["kmxSourceCommit"] + "/", fixture["url"])

    def test_sources_refuse_unpinned_or_escaping_rows(self):
        sources, _ = synthetic_sources()
        changes = [("url", "http://example.test/base"), ("path", "../base"), ("path", "/base"), ("bytes", 0), ("bytes", True),
                   ("sha256", "0" * 63), ("use", "script")]
        for field, value in changes:
            with self.subTest(field=field, value=value), tempfile.TemporaryDirectory() as temp:
                changed = json.loads(json.dumps(sources))
                changed["files"][0][field] = value
                path = Path(temp) / "sources.json"
                path.write_text(json.dumps(changed))
                with self.assertRaises(ValueError):
                    ci_inputs.load_sources(path)

    def test_sources_refuse_overlapping_paths_and_an_unpinned_chart(self):
        for mutate in (lambda s: s["files"][1].update(path="base/debian.qcow2/x"),
                       lambda s: s["files"][3].update(version=COMMIT)):
            sources, _ = synthetic_sources()
            mutate(sources)
            with tempfile.TemporaryDirectory() as temp:
                path = Path(temp) / "sources.json"
                path.write_text(json.dumps(sources))
                with self.assertRaises(ValueError):
                    ci_inputs.load_sources(path)


class FetchTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.out = Path(temp.name) / "downloads"
        self.sources, self.payloads = synthetic_sources()

    def fetch(self, server, **kwargs):
        with patch("builtins.print") as printed:
            ci_inputs.fetch(self.sources, self.out, opener=server, pause=lambda _: None, **kwargs)
        return [call.args[0].split(" (")[0] for call in printed.call_args_list]

    def test_verified_files_are_kept_and_damaged_files_replaced(self):
        server = Server(self.payloads)
        self.assertIn("fetched tools/kind", self.fetch(server))
        self.assertEqual(4, len(server.requests))
        for path, data in self.payloads.items():
            self.assertEqual(data, (self.out / path).read_bytes())
        server.requests.clear()
        self.assertIn("kept tools/kind", self.fetch(server))
        self.assertEqual([], server.requests)
        (self.out / "tools/kind").write_bytes(b"synthetic kinD")
        self.assertIn("replaced tools/kind", self.fetch(server))
        self.assertEqual(["https://example.test/tools/kind"], server.requests)
        self.assertEqual(self.payloads["tools/kind"], (self.out / "tools/kind").read_bytes())

    def test_wrong_bytes_are_refused_after_retries_without_a_file(self):
        for wrong in (b"synthetic kinD", b"synthetic kind!", b"short"):
            with self.subTest(wrong=wrong):
                server = Server(self.payloads, {"tools/kind": wrong})
                with self.assertRaisesRegex(ValueError, "tools/kind"):
                    self.fetch(server, attempts=2)
                self.assertEqual(2, server.requests.count("https://example.test/tools/kind"))
                self.assertFalse((self.out / "tools/kind").exists())
                self.assertEqual([], list(self.out.glob("**/*.partial")))

    def test_transient_failure_is_retried(self):
        server = Server(self.payloads)
        calls = []

        def flaky(url, timeout):
            calls.append(url)
            if len(calls) == 1:
                raise TimeoutError("timed out")
            return server(url, timeout)
        self.fetch(flaky)
        self.assertEqual(self.payloads["base/debian.qcow2"], (self.out / "base/debian.qcow2").read_bytes())

    def test_cached_symlink_is_refused(self):
        outside = self.out.parent / "outside"
        outside.mkdir()
        self.out.mkdir()
        (self.out / "tools").symlink_to(outside)
        with self.assertRaisesRegex(ValueError, "symlink"):
            self.fetch(Server(self.payloads))
        self.assertEqual([], list(outside.iterdir()))

    def test_cached_root_symlink_is_refused(self):
        outside = self.out.parent / "outside"
        (outside / "tools").mkdir(parents=True)
        (outside / "tools/kind").write_bytes(b"unchanged")
        self.out.symlink_to(outside)
        with self.assertRaisesRegex(ValueError, "symlink"):
            self.fetch(Server(self.payloads))
        self.assertEqual(b"unchanged", (outside / "tools/kind").read_bytes())
        self.assertEqual(["tools"], [p.name for p in outside.iterdir()])

    def test_cached_partial_symlink_is_not_followed(self):
        outside = self.out.parent / "outside"
        outside.write_bytes(b"unchanged")
        (self.out / "tools").mkdir(parents=True)
        (self.out / "tools/kind.partial").symlink_to(outside)
        self.fetch(Server(self.payloads))
        self.assertEqual(b"unchanged", outside.read_bytes())
        self.assertEqual(self.payloads["tools/kind"], (self.out / "tools/kind").read_bytes())
        self.assertFalse((self.out / "tools/kind.partial").exists() or (self.out / "tools/kind.partial").is_symlink())


class AssembleTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        self.downloads, self.runtime, self.out, self.bin = root / "downloads", root / "runtime", root / "inputs", root / "bin"
        self.sources, payloads = synthetic_sources()
        for path, data in payloads.items():
            (self.downloads / path).parent.mkdir(parents=True, exist_ok=True)
            (self.downloads / path).write_bytes(data)
        self.runtime.mkdir()
        binary = b"\x7fELF\x02\x01" + bytes(12) + struct.pack("<H", 62) + bytes(64)
        (self.runtime / "bridge-guest").write_bytes(binary)
        provenance = {"schemaVersion": "bridge-guest-binary/v1", "sourceCommit": COMMIT, "target": "x86_64-unknown-linux-musl",
                      "binarySha256": hashlib.sha256(binary).hexdigest(), "binaryBytes": len(binary), "contractVersion": "H1/v1",
                      "protocolMajor": 1}
        (self.runtime / "provenance.json").write_text(json.dumps(provenance))
        self.bin.mkdir()
        for name in ci_inputs.BUILDER_TOOLS:
            tool = self.bin / name
            tool.write_text("#!/bin/sh\necho '" + name + " 1.0'\n")
            tool.chmod(0o755)
        self.chart = patch.dict(VERSIONS["sourceOfflineProfile"], chartSha256=hashlib.sha256(CHART).hexdigest())
        self.chart.start()
        self.addCleanup(self.chart.stop)

    def assemble(self):
        with patch.dict(os.environ, PATH=str(self.bin) + os.pathsep + os.environ["PATH"]):
            return ci_inputs.assemble(self.sources, self.downloads, self.runtime, self.out)

    def test_lock_is_a_base_lock_that_assembly_accepts(self):
        lock = self.assemble()
        self.assertEqual(lock, load_lock(self.out / "inputs.json"))
        self.assertIsNone(lock["kmxProfileSha256"])
        self.assertEqual(CHART, (self.out / "chart/orka.tgz").read_bytes())
        tools = {t["name"]: t for t in lock["tools"]}
        self.assertNotIn("kmx-offline-manifest", tools)
        self.assertEqual(COMMIT, tools["bridge-guest"]["version"])
        for name in ci_inputs.BUILDER_TOOLS:
            self.assertEqual(name + " 1.0", tools[name]["version"])
            self.assertEqual(file_hash(self.bin / name), tools[name]["sha256"])
        self.assertEqual({"debian-base", "tini"}, {p["name"] for p in lock["packages"]})
        for component in lock["components"]:
            self.assertTrue(component["repository"].startswith("localhost/bridge-ci-unused/"))
            self.assertFalse((self.out / component["archive"]).exists())
        self.assertFalse((self.out / "kmx").exists())

    def test_retained_chart_must_match_the_pinned_profile(self):
        self.chart.stop()
        with self.assertRaisesRegex(ValueError, "retained chart"):
            self.assemble()
        self.chart.start()

    def test_changed_download_is_refused(self):
        (self.downloads / "debs/tini.deb").write_bytes(b"synthetic deB")
        with self.assertRaisesRegex(ValueError, "checksum"):
            self.assemble()

    def test_endpoint_provenance_must_match_its_binary(self):
        (self.runtime / "bridge-guest").write_bytes(b"\x7fELF\x02\x01" + bytes(100))
        with self.assertRaisesRegex(ValueError, "provenance"):
            self.assemble()

    def test_missing_builder_tool_is_refused(self):
        (self.bin / "virt-resize").unlink()
        with patch.dict(os.environ, PATH=str(self.bin)), self.assertRaisesRegex(ValueError, "virt-resize"):
            ci_inputs.assemble(self.sources, self.downloads, self.runtime, self.out)


if __name__ == "__main__":
    unittest.main()
