"""Local source-input contract tests; no downloads, compilers or guests."""
import contextlib
import copy
import hashlib
import io
import json
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any
from unittest.mock import patch

import recipe

REFERENCE = "a" * 40
QEMU = "b" * 40
PATCHES = ("0001", "0005", "0009", "0015", "0019", "0024")
CHECKS = ("test-socket-close.py", "test-fd-setsize.py", "test-whpx-cpu-model.py")


def fixture():
    def entry(path, origin):
        data = path.encode()
        return {"path": path, "url": origin, "bytes": len(data),
                "sha256": hashlib.sha256(data).hexdigest()}
    ref = f"https://raw.githubusercontent.com/omacom/try-omarchy-windows/{REFERENCE}/runtime-build/"
    qemu = f"https://raw.githubusercontent.com/cmspam/winq-emu-qemu/{QEMU}/"
    return {"schemaVersion": "bridge-windows-sources/v1", "sourceDateEpoch": 1777269494,
            "reference": {"repository": "https://github.com/omacom/try-omarchy-windows", "commit": REFERENCE},
            "qemu": {"repository": "https://github.com/cmspam/winq-emu-qemu", "commit": QEMU, "baseCommit": "c" * 40},
            "patches": [entry(f"patches/qemu/{n}-fixture.patch", ref + f"patches/qemu/{n}-fixture.patch") for n in PATCHES],
            "checks": [entry(f"checks/{n}", ref + n) for n in CHECKS],
            "wraps": [entry("subprojects/example.wrap", qemu + "subprojects/example.wrap")],
            "submodules": [{"path": "roms/seabios", "repository": "https://gitlab.com/qemu-project/seabios.git", "commit": "d" * 40}]}


def source_metadata(record):
    member = {"bytes": 1, "sha256": "a" * 64}
    return {**record, "version": "1-1", "recipeMembers": [{"path": "PKGBUILD", **member}, {"path": ".SRCINFO", **member}],
            "sourceReferences": [{"present": True, "vcs": False, "archiveMembers": [{"path": "source.tar", **member}]}],
            "notices": [{"bytes": 10, "sha256": hashlib.sha256(b"MIT notice").hexdigest()}]}


class Sources(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.lock = self.root / "sources.json"

    def load(self, value=None):
        self.lock.write_text(json.dumps(value if value is not None else fixture()), encoding="utf-8")
        return recipe.load_sources(self.lock)

    def stage(self, lock):
        inputs = self.root / "inputs"
        inputs.mkdir()
        for group in ("patches", "checks", "wraps"):
            for item in lock[group]:
                path = inputs / item["path"]
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(item["path"].encode())
        return inputs

    def test_valid_lock_and_input_bytes(self):
        lock = self.load()
        inputs = self.stage(lock)
        self.assertEqual(recipe.check_sources(lock, inputs), 10)

    def test_missing_unknown_and_bad_fields(self):
        mutations = [lambda d: d.pop("qemu"), lambda d: d.update(extra=1),
                     lambda d: d.update(sourceDateEpoch=True),
                     lambda d: d["qemu"].update(commit="main"),
                     lambda d: d["patches"][0].update(sha256="0" * 63),
                     lambda d: d["patches"][0].update(bytes=True)]
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                data = fixture()
                mutate(data)
                with self.assertRaises(ValueError):
                    self.load(data)

    def test_patch_bytes_and_order(self):
        for change in (lambda d: d["patches"].reverse(), lambda d: d["patches"].pop(),
                       lambda d: d["checks"].pop(), lambda d: d["patches"].append(d["patches"][0])):
            data = fixture()
            change(data)
            with self.assertRaises(ValueError):
                self.load(data)

    def test_immutable_origins(self):
        for group in ("patches", "checks", "wraps"):
            data = fixture()
            data[group][0]["url"] = "https://example.com/latest"
            with self.assertRaises(ValueError):
                self.load(data)

    def test_repository_has_no_record_separator(self):
        data = fixture()
        data["submodules"][0]["repository"] = "https://github.com/example/repo\nother-record"
        with self.assertRaises(ValueError):
            self.load(data)

    def test_windows_path_aliases(self):
        for name in ("../x", "/x", "C:/x", "a\\b", "a//b", "a/./b", "a/NUL", "a/COM1.txt", "a/b.", "a/b ", "a/b:c"):
            data = fixture()
            data["wraps"][0]["path"] = name
            data["wraps"][0]["url"] = f"https://raw.githubusercontent.com/cmspam/winq-emu-qemu/{QEMU}/{name}"
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, "unsafe source path"):
                self.load(data)

    def test_duplicate_check_basename(self):
        data = fixture()
        duplicate = copy.deepcopy(data["checks"][0])
        duplicate["path"] = "checks/extra/test-socket-close.py"
        duplicate["url"] = duplicate["url"].replace("/test-socket-close.py", "/extra/test-socket-close.py")
        data["checks"].append(duplicate)
        with self.assertRaisesRegex(ValueError, "required source checks"):
            self.load(data)

    def test_duplicate_keys_and_paths(self):
        self.lock.write_text('{"schemaVersion":1,"schemaVersion":2}', encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "duplicate"):
            recipe.load_sources(self.lock)
        data = fixture()
        data["submodules"].append(copy.deepcopy(data["submodules"][0]))
        with self.assertRaises(ValueError):
            self.load(data)

    def test_corrupt_missing_and_nonregular_inputs(self):
        lock = self.load()
        inputs = self.stage(lock)
        path = inputs / lock["patches"][0]["path"]
        path.write_bytes(b"corrupt")
        with self.assertRaisesRegex(ValueError, "0001"):
            recipe.check_sources(lock, inputs)
        path.unlink()
        with self.assertRaises(ValueError):
            recipe.check_sources(lock, inputs)
        path.mkdir()
        with self.assertRaises(ValueError):
            recipe.check_sources(lock, inputs)

    def test_equal_length_corruption(self):
        lock = self.load()
        inputs = self.stage(lock)
        path = inputs / lock["wraps"][0]["path"]
        path.write_bytes(b"x" * path.stat().st_size)
        with self.assertRaisesRegex(ValueError, "sha256"):
            recipe.check_sources(lock, inputs)

    def test_symlink_component(self):
        lock = self.load()
        inputs = self.stage(lock)
        folder = inputs / "subprojects"
        file = folder / "example.wrap"
        target = self.root / "example.wrap"
        file.replace(target)
        folder.rmdir()
        try:
            folder.symlink_to(self.root, target_is_directory=True)
        except OSError:
            self.skipTest("host does not grant symlink creation")
        with self.assertRaises(ValueError):
            recipe.check_sources(lock, inputs)

    def test_cli_distinguishes_structure_from_bytes(self):
        self.load()
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            self.assertEqual(recipe.main(["check-sources", "--lock", str(self.lock)]), 0)
        result = json.loads(output.getvalue())
        self.assertFalse(result["publicationReady"])
        self.assertFalse(result["inputBytesVerified"])
        self.assertEqual(result["filesVerified"], 0)
        inputs = self.stage(self.load())
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            recipe.main(["check-sources", "--lock", str(self.lock), "--inputs", str(inputs)])
        self.assertTrue(json.loads(output.getvalue())["inputBytesVerified"])

    def test_cli_failure_has_no_success_json(self):
        self.load()
        with contextlib.redirect_stdout(io.StringIO()) as out, contextlib.redirect_stderr(io.StringIO()) as err:
            code = recipe.main(["check-sources", "--lock", str(self.lock), "--inputs", str(self.root / "absent")])
        self.assertEqual(code, 1)
        self.assertEqual(out.getvalue(), "")
        self.assertIn("source inputs", err.getvalue())

    def test_checked_in_lock_and_no_execution(self):
        with patch("subprocess.run", side_effect=AssertionError("no subprocess")), patch("subprocess.check_output", side_effect=AssertionError("no subprocess")):
            lock = recipe.load_sources(Path(__file__).with_name("sources.lock.json"))
        self.assertEqual(len(lock["patches"]), 6)
        self.assertEqual(len(lock["checks"]), 3)

    def test_subprocess_cli(self):
        self.load()
        result = subprocess.run([sys.executable, str(Path(recipe.__file__)), "check-sources", "--lock", str(self.lock)], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(json.loads(result.stdout)["inputBytesVerified"])


class Workflow(unittest.TestCase):
    def test_producer_build_authority(self):
        root = Path(__file__).resolve().parents[2]
        text = (root / ".github/workflows/windows-runtime.yml").read_text()
        self.assertIn("timeout-minutes: 120", text)
        self.assertIn("build.sh", text)
        self.assertIn("System32", text)
        self.assertIn("--version", text)
        script = (root / "runtime/windows/build.sh").read_text()
        for required in ("--disable-download", "--disable-tcg", "--enable-whpx", "--audio-drv-list=", "git apply --check", "git apply --index"):
            self.assertIn(required, script)
        for forbidden in ("pull_request_target", "contents: write", "secrets.", "upload-artifact", "-kernel", "-drive", "-Syu", "setup-msys2"):
            self.assertNotIn(forbidden, text + script)

    def test_read_only_path_filtered_source_validation(self):
        root = Path(__file__).resolve().parents[2]
        text = (root / ".github/workflows/windows-runtime.yml").read_text(encoding="utf-8")
        self.assertIn("pull_request:", text)
        self.assertIn("push:", text)
        self.assertEqual(text.count("- 'runtime/windows/**'"), 2)
        self.assertEqual(text.count("- '.github/workflows/windows-runtime.yml'"), 2)
        self.assertIn("contents: read", text)
        self.assertIn("persist-credentials: false", text)
        self.assertIn("github.event.pull_request.head.sha || github.sha", text)
        self.assertIn("GIT_CONFIG_KEY_0: core.autocrlf", text)
        self.assertIn("GIT_CONFIG_VALUE_0: 'false'", text)
        self.assertIn("cancel-in-progress: true", text)
        self.assertIn("windows-2025", text)
        self.assertIn("check-sources --lock runtime/windows/sources.lock.json", text)
        self.assertIn("python -m unittest discover -s runtime/windows", text)
        for forbidden in ("pull_request_target", "contents: write", "secrets.", "upload-artifact", "workflow_dispatch"):
            self.assertNotIn(forbidden, text)
        for action in re.findall(r"uses: ([^\s]+)", text):
            self.assertRegex(action, r"@[a-f0-9]{40}$")


class Packages(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "sources.json"
        self.source.write_text(json.dumps(fixture()))
        self.path = self.root / "packages.json"
        def archive(name, data):
            return {"path": name, "url": "https://repo.msys2.org/" + name,
                    "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(),
                    "provenance": {"url": "https://repo.msys2.org/msys/x86_64/msys.db",
                                   "sha256": "a" * 64, "method": "official-repository-database"}}
        self.data: dict[str, Any] = {"schemaVersion": "bridge-windows-packages/v1",
                     "bootstrap": archive("bootstrap/base.tar.xz", b"base"),
                     "referenceNotice": archive("reference/LICENSE", b"reference"),
                     "bootstrapPackages": [{"name": "base", "version": "1-1"}],
                     "packages": [{**archive("packages/lib.pkg.tar.zst", b"lib"),
                                   "name": "lib", "version": "1-1", "license": ["MIT"],
                                   "redistribution": "runtime",
                                   "source": source_metadata(archive("sources/lib.src.tar.zst", b"source"))}],
                     "pythonArchives": [], "expectedInstalled": [{"name": "base", "version": "1-1"},
                                                                  {"name": "lib", "version": "1-1"}],
                     "firmware": [], "sourceTrees": {"qemu": "a" * 40, "qemu/roms/seabios": "a" * 40}}

    def load(self):
        self.path.write_text(json.dumps(self.data))
        return recipe.load_locks(self.source, self.path)

    def test_required_recipe_and_distfile_metadata(self):
        original = copy.deepcopy(self.data)
        for change in (lambda s: s.update(version="different"), lambda s: s.pop("version"),
                       lambda s: s["recipeMembers"].pop(0),
                       lambda s: s["sourceReferences"][0].update(present=False),
                       lambda s: s["sourceReferences"][0].update(archiveMembers=[]),
                       lambda s: s.update(notices=[])):
            self.data = copy.deepcopy(original)
            change(self.data["packages"][0]["source"])
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.load()

    def test_git_snapshot_preserves_blobs_not_git_database(self):
        checkout = self.root / "checkouts/qemu"
        checkout.mkdir(parents=True)
        content = b"fn main() {\n}\n"
        (checkout / "fork.rs").write_bytes(content)
        (checkout / ".git").mkdir()
        (checkout / ".git/config").write_text("not source material")
        oid = hashlib.sha1(f"blob {len(content)}\0".encode() + content).hexdigest()
        def output(args):
            if "ls-files" in args:
                return f"120000 {oid} 0\tfork.rs\0"
            if "diff-files" in args:
                return ""
            return ("b" if args[-1] == "HEAD" else "a") * 40 + "\n"
        rows = [{"path": "qemu", "repository": "https://github.com/cmspam/winq-emu-qemu", "commit": "b" * 40}]
        dest = self.root / "source"
        with patch("recipe.output", side_effect=output):
            recipe.snapshot(self.root / "checkouts", dest, rows)
        self.assertEqual((dest / "qemu/fork.rs").read_bytes(), content)
        self.assertFalse((dest / "qemu/.git").exists())
        index = json.loads((dest / "provenance/git-index.json").read_text())
        self.assertEqual(index["files"]["qemu/fork.rs"]["mode"], "120000")
        self.assertEqual(index["files"]["qemu/fork.rs"]["oid"], oid)
        (checkout / "fork.rs").write_bytes(b"corrupt")
        with patch("recipe.output", side_effect=output), self.assertRaisesRegex(ValueError, "blob"):
            recipe.snapshot(self.root / "checkouts", self.root / "bad-source", rows)

    def test_effective_features_reject_unknown_and_missing(self):
        import verify
        options: dict[str, Any] = {k: ("enabled" if v else "disabled") if type(v) is bool else v for k, v in verify.FEATURES.items()}
        self.assertEqual(recipe.effective_features(options), verify.FEATURES)
        for value in ("auto", "unknown", 0, None):
            options["tcg"] = value
            with self.subTest(value=value), self.assertRaises(ValueError):
                recipe.effective_features(options)
        options.pop("tcg")
        with self.assertRaises(ValueError):
            recipe.effective_features(options)

    def test_compiled_features_accept_meson_boolean_forms(self):
        configs = "#define CONFIG_WHPX 1\n#define CONFIG_SLIRP\n#define CONFIG_ZSTD\n#define CONFIG_PIXMAN\n#undef CONFIG_TCG\n#define CONFIG_PLUGIN 0\n"
        recipe.check_compiled_features(configs)
        for name in ("TCG", "PLUGIN", "AUDIO_ALSA", "AUDIO_SDL"):
            for value in ("", " 1"):
                with self.subTest(name=name, value=value), self.assertRaisesRegex(ValueError, "forbidden compiled feature"):
                    recipe.check_compiled_features(configs.replace("#define CONFIG_PLUGIN 0\n", "") + f"#define CONFIG_{name}{value}\n")
        for value in ("#undef CONFIG_SLIRP", "#define CONFIG_SLIRP 0", ""):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "compiled feature missing: SLIRP"):
                recipe.check_compiled_features(configs.replace("#define CONFIG_SLIRP", value))
        for value in ("2", "auto", "(1)"):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "unknown/conflicting compiled feature"):
                recipe.check_compiled_features(configs + f"#define CONFIG_PLUGIN {value}\n")
        with self.assertRaisesRegex(ValueError, "unknown/conflicting compiled feature"):
            recipe.check_compiled_features(configs + "#define CONFIG_SLIRP 0\n")

    def test_import_resolution(self):
        text = "sample: file format pei-x86-64\n\tDLL Name: KERNEL32.dll\n\tDLL Name: demo.dll\n"
        self.assertEqual(recipe.import_names(text), ["KERNEL32.dll", "demo.dll"])
        self.assertIsNone(recipe.resolve_import("kernel32.dll", {}))
        dll = Path("/ucrt64/bin/demo.dll")
        self.assertEqual(recipe.resolve_import("DEMO.DLL", recipe.dll_table([dll])), dll)
        with self.assertRaises(ValueError):
            recipe.resolve_import("missing.dll", {})
        with self.assertRaises(ValueError):
            recipe.dll_table([dll, Path("/other/DEMO.dll")])
        with self.assertRaises(ValueError):
            recipe.import_names(text.replace("pei-x86-64", "pei-i386"))
        with self.assertRaises(ValueError):
            recipe.import_names(text.replace("demo.dll", "../demo.dll"))

    def test_checked_in_installed_qemu_data_notices(self):
        packages = recipe.read_json(Path(__file__).with_name("packages.lock.json"))
        rows = {r["path"]: r for r in packages["firmware"]}
        artwork = [f"icons/hicolor/{size}x{size}/apps/qemu.png" for size in (16, 24, 32, 48, 64, 128, 256, 512)]
        artwork += ["icons/hicolor/32x32/apps/qemu.bmp", "icons/hicolor/scalable/apps/qemu.svg"]
        for path in artwork:
            self.assertEqual(rows["share/qemu/" + path], {"path": "share/qemu/" + path, "source": "qemu", "license": "qemu/ui/icons/qemu.svg"})
        for path in ("applications/qemu.desktop", "trace-events-all"):
            self.assertEqual(rows["share/qemu/" + path]["license"], "qemu/LICENSE")
        self.assertEqual(len(rows), 127)
        notices = {r["path"]: r for r in packages["firmwareNotices"]}
        self.assertEqual(notices["qemu/ui/icons/qemu.svg"]["gitBlob"], "f2500de339170ae69cb27ec3c02464d187cf1359")

    def test_real_package_paths(self):
        for path in ("packages/libstdc++-1-1.pkg.tar.zst", "packages/pkgconf-1~3.0-1.pkg.tar.zst"):
            self.data["packages"][0]["path"] = path
            self.load()

    def test_package_cli_structure_and_inventory(self):
        self.load()
        for command in ("check-locks", "list-inputs"):
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                code = recipe.main([command, "--sources", str(self.source), "--packages", str(self.path)])
            self.assertEqual(code, 0)
            result = json.loads(out.getvalue())
            if command == "list-inputs":
                self.assertTrue(any(r["path"] == "sources/lib.src.tar.zst" for r in result))
            else:
                self.assertFalse(result["publicationReady"])
        inventory = self.root / "inventory.json"
        inventory.write_text(json.dumps(self.data["expectedInstalled"]))
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(recipe.main(["check-installed", "--packages", str(self.path), "--inventory", str(inventory)]), 0)

    def test_installed_inventory(self):
        _, packages = self.load()
        recipe.check_installed(packages, self.data["expectedInstalled"])
        for rows in ([{"name": "lib", "version": "1-1"}],
                     [*self.data["expectedInstalled"], {"name": "extra", "version": "1"}],
                     [*self.data["expectedInstalled"], self.data["expectedInstalled"][0]],
                     [{"name": "base", "version": "2"}, {"name": "lib", "version": "1-1"}]):
            with self.subTest(rows=rows), self.assertRaises(ValueError):
                recipe.check_installed(packages, rows)

    def test_missing_pin_source_and_duplicate_package(self):
        original = copy.deepcopy(self.data)
        for change in (lambda d: d["bootstrap"].update(sha256=None),
                       lambda d: d["packages"][0].pop("source"),
                       lambda d: d["packages"].append(d["packages"][0]),
                       lambda d: d["expectedInstalled"].pop(),
                       lambda d: d["packages"][0].update(path="../outside")):
            self.data = copy.deepcopy(original)
            change(self.data)
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.load()

    def test_input_closure(self):
        sources, packages = self.load()
        for group in recipe.GROUPS:
            for item in sources[group]:
                path = self.root / item["path"]
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(item["path"].encode())
        for item, data in ((packages["bootstrap"], b"base"), (packages["referenceNotice"], b"reference"),
                           (packages["packages"][0], b"lib"),
                           (packages["packages"][0]["source"], b"source")):
            path = self.root / item["path"]
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        recipe.check_inputs(sources, packages, self.root)
        (self.root / "sources/lib.src.tar.zst").write_bytes(b"corrupt")
        with self.assertRaises(ValueError):
            recipe.check_inputs(sources, packages, self.root)


if __name__ == "__main__":
    unittest.main()
