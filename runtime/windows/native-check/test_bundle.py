"""Exercise bundle trust boundaries with tiny real producer archives, no downloads."""
import contextlib
import hashlib
import io
import json
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import test_verify

import bundle


class BundleTests(unittest.TestCase):
    def setUp(self):
        self.fixture = test_verify.Archives()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.fixture.build()
        self.root = self.fixture.root
        self.run = {
            "databaseId": 42120608528,
            "headSha": "006b30f02a3c9cec61f8f3cc32fde6b0d248fc34",
            "headBranch": "main", "event": "workflow_dispatch",
            "workflowName": "Windows runtime", "status": "completed", "conclusion": "success",
        }
        self.run_path = self.root / "producer-run.json"
        self.run_path.write_text(json.dumps(self.run))
        self.exe = self.root / "bridge-native-premises.exe"
        self.exe.write_bytes(b"fixture executable; never executed")
        self.operator = self.root / "operator"
        self.runtime = self.root / "extracted"

    def prepare(self):
        return bundle.prepare(
            self.fixture.output, self.run_path, self.fixture.source_lock,
            self.fixture.package_lock, self.exe, self.operator, self.runtime, "a" * 40)

    def test_inventory_includes_manifest_itself_and_preserves_archives(self):
        self.prepare()
        pin = json.loads((self.operator / "runtime-pin.json").read_text())
        self.assertEqual(set(pin), {"head", "producerRun", "producerHead", "runtimeZipSha256", "runtimeFiles"})
        self.assertEqual(pin["head"], "423c0abdb4070a1a693493b405069845bb7f749f")
        self.assertEqual(pin["producerRun"], 42120608528)
        self.assertEqual(pin["producerHead"], self.run["headSha"])
        names = [row["name"] for row in pin["runtimeFiles"]]
        self.assertEqual(names, sorted(names))
        self.assertIn("provenance/runtime-manifest.json", names)
        self.assertEqual(set(names), {p.relative_to(self.runtime).as_posix()
                                     for p in self.runtime.rglob("*") if p.is_file()})
        for row in pin["runtimeFiles"]:
            data = (self.runtime / row["name"]).read_bytes()
            self.assertEqual(row, {"name": row["name"], "bytes": len(data),
                                   "sha256": hashlib.sha256(data).hexdigest()})
        for name in ("bridge-windows-runtime-portable.zip", "bridge-windows-runtime-source.zip",
                     "SHA256SUMS", "build-report.json", "verification.json"):
            self.assertEqual((self.operator / name).read_bytes(), (self.fixture.output / name).read_bytes())

    def test_wrong_or_unsuccessful_producer_never_extracts(self):
        for key, wrong in (("databaseId", 1), ("headSha", "b" * 40), ("conclusion", "failure"),
                           ("status", "in_progress"), ("headBranch", "other"),
                           ("event", "push"), ("workflowName", "Other")):
            with self.subTest(key=key):
                self.run_path.write_text(json.dumps({**self.run, key: wrong}))
                with self.assertRaises(ValueError):
                    self.prepare()
                self.assertFalse(self.runtime.exists())
                self.assertFalse(self.operator.exists())

    def test_source_archive_corruption_is_rejected_before_extraction(self):
        path = self.fixture.output / "bridge-windows-runtime-source.zip"
        path.write_bytes(path.read_bytes() + b"corrupt")
        with self.assertRaisesRegex(ValueError, "SHA256SUMS mismatch"):
            self.prepare()
        self.assertFalse(self.runtime.exists())

    def test_unsafe_runtime_entry_is_rejected_before_extraction(self):
        def add_escape(entries):
            import stat
            import zipfile
            entry = zipfile.ZipInfo("../escape")
            entry.external_attr = (stat.S_IFREG | 0o644) << 16
            return entries + [(entry, b"outside")]
        self.fixture.rewrite("bridge-windows-runtime-portable.zip", add_escape)
        with self.assertRaisesRegex(ValueError, "unsafe archive path"):
            self.prepare()
        self.assertFalse(self.runtime.exists())
        self.assertFalse((self.root / "escape").exists())

    def test_external_lock_change_is_rejected_before_extraction(self):
        self.fixture.sources["qemu"]["commit"] = "b" * 40
        self.fixture.source_lock.write_text(json.dumps(self.fixture.sources))
        with self.assertRaises(ValueError):
            self.prepare()
        self.assertFalse(self.runtime.exists())

    def test_existing_runtime_directory_is_not_touched(self):
        self.runtime.mkdir()
        (self.runtime / "keep").write_text("unchanged")
        with self.assertRaises(FileExistsError):
            self.prepare()
        self.assertEqual((self.runtime / "keep").read_text(), "unchanged")

    def test_hash_list_contains_only_actual_allowlisted_files(self):
        self.prepare()
        (self.operator / "ci-stdout.txt").write_text("measured output")
        (self.operator / "unrelated-secret").write_text("must not be included")
        with contextlib.redirect_stdout(io.StringIO()):
            bundle.finish(self.operator)
        sums = (self.operator / "operator-sha256sums.txt").read_text().splitlines()
        records = dict(line.split("  ", 1)[::-1] for line in sums)
        self.assertNotIn("unrelated-secret", records)
        self.assertNotIn("operator-sha256sums.txt", records)
        self.assertIn("ci-stdout.txt", records)
        self.assertIn("bridge-native-premises.exe", records)
        for name, digest in records.items():
            self.assertEqual(digest, hashlib.sha256((self.operator / name).read_bytes()).hexdigest())

    def test_every_manifest_candidate_is_retained_by_workflow(self):
        workflow = (HERE.parents[2] / ".github/workflows/windows-native-check.yml").read_text()
        upload_paths = workflow.split("          path: |\n", 1)[1].split("          retention-days:", 1)[0]
        for name in (*bundle.BUNDLE_FILES, "operator-sha256sums.txt"):
            with self.subTest(name=name):
                self.assertIn(f"/native-operator/{name}\n", upload_paths)

    def test_symlink_in_operator_allowlist_is_rejected(self):
        self.operator.mkdir()
        try:
            (self.operator / "ci-stdout.txt").symlink_to(self.exe)
        except OSError:
            self.skipTest("symlink creation unavailable")
        with self.assertRaises(ValueError):
            bundle.finish(self.operator)


if __name__ == "__main__":
    unittest.main()
