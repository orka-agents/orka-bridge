"""Producer archives are checked against external locks, without extraction."""
import copy
import hashlib
import json
import struct
import tempfile
import unittest
import zipfile
from pathlib import Path
from typing import Any
from unittest.mock import patch

import recipe
import verify
from test_recipe import fixture, source_metadata


def encoded(value):
    return (json.dumps(value, sort_keys=True, indent=2) + "\n").encode()


def archive_record(path, data):
    return {"path": path, "url": "https://repo.msys2.org/mingw/ucrt64/" + path.split("/")[-1],
            "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(),
            "provenance": {"url": "https://repo.msys2.org/mingw/ucrt64/ucrt64.db",
                           "sha256": "a" * 64, "method": "official-package-database"}}


def pe(machine=0x8664):
    data = bytearray(256)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 60, 128)
    data[128:132] = b"PE\0\0"
    struct.pack_into("<H", data, 132, machine)
    struct.pack_into("<H", data, 152, 0x20b)
    return bytes(data)


class Archives(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.runtime, self.source = self.root / "runtime", self.root / "source"
        self.runtime.mkdir()
        self.source.mkdir()
        self.sources = fixture()
        package = {"name": "lib", "version": "1-1", "license": ["MIT"], "redistribution": "runtime",
                   **archive_record("packages/lib.pkg.tar.zst", b"package"),
                   "source": source_metadata(archive_record("sources/lib.src.tar.zst", b"source"))}
        self.packages: dict[str, Any] = {"schemaVersion": "bridge-windows-packages/v1",
                         "bootstrap": archive_record("bootstrap/base.tar.xz", b"base"),
                         "referenceNotice": archive_record("reference/LICENSE", b"reference"),
                         "bootstrapPackages": [{"name": "base", "version": "1-1"}],
                         "packages": [package], "pythonArchives": [],
                         "policyPatch": {"path": "disable-dsound.patch", "bytes": 12,
                                         "sha256": hashlib.sha256(b"policy patch").hexdigest()},
                         "expectedInstalled": [{"name": "base", "version": "1-1"},
                                               {"name": "lib", "version": "1-1"}],
                         "firmware": [{"path": "share/qemu/bios-256k.bin", "source": "qemu", "license": "qemu/COPYING"}]}
        self.source_lock, self.package_lock = self.root / "sources.json", self.root / "packages.json"
        self.source_lock.write_bytes(encoded(self.sources))
        self.package_lock.write_bytes(encoded(self.packages))
        for group in recipe.GROUPS:
            for item in self.sources[group]:
                self.put(self.source, "inputs/" + item["path"], item["path"].encode())
        self.put(self.source, "inputs/sources/lib.src.tar.zst", b"source")
        for name in ("COPYING", "COPYING.LIB", "LICENSE"):
            self.put(self.source, "qemu/" + name, b"license")
            self.put(self.runtime, "LICENSES/qemu/" + name, b"license")
        self.put(self.source, "recipe/build.sh", b"#!/bin/bash\n")
        self.put(self.source, "recipe/disable-dsound.patch", b"policy patch")
        for name in ("recipe.py", "verify.py", "README.md", "windows-runtime.yml"):
            self.put(self.source, "recipe/" + name, b"fixture")
        self.put(self.source, "inputs/reference/LICENSE", b"reference")
        self.put(self.runtime, "LICENSES/reference/LICENSE", b"reference")
        self.put(self.runtime, "LICENSES/msys2/lib/COPYING", b"MIT notice")
        self.put(self.runtime, "share/qemu/bios-256k.bin", b"bios")
        self.exes = ["bin/qemu-system-x86_64.exe", "bin/qemu-img.exe"]
        for name in [*self.exes, "bin/demo.dll"]:
            self.put(self.runtime, name, pe())
        imports = {p: [{"name": "KERNEL32.dll", "system": True},
                        {"name": "demo.dll", "system": False, "path": "bin/demo.dll", "owner": "lib"}]
                   for p in self.exes}
        imports["bin/demo.dll"] = []
        self.evidence: dict[str, Any] = {"sourceCommit": self.sources["qemu"]["commit"],
                         "lockSha256": {"sources": hashlib.sha256(self.source_lock.read_bytes()).hexdigest(),
                                        "packages": hashlib.sha256(self.package_lock.read_bytes()).hexdigest()},
                         "installed": self.packages["expectedInstalled"], "imports": imports,
                         "firmware": self.packages["firmware"],
                         "features": {"whpx": True, "slirp": True, "tools": True, "zstd": True, "pixman": True,
                                      "tcg": False, "plugins": False, "sdl": False, "gtk": False, "vnc": False,
                                      "curses": False, "opengl": False, "virglrenderer": False, "rutabaga_gfx": False,
                                      "libusb": False, "usb_redir": False, "virtfs": False, "guest_agent": False,
                                      "audio_drv_list": []},
                         "tools": {"compiler": "fixture", "runner": "fixture"}}
        self.output = self.root / "output"
        self.git_modes = {}
        self.put(self.source, "qemu/roms/seabios/COPYING", b"firmware source")

    def put(self, root, name, data):
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)

    def build(self):
        files = {}
        for path in self.source.rglob("*"):
            if path.is_file() or path.is_symlink():
                name = path.relative_to(self.source).as_posix()
                if name.startswith("qemu/"):
                    data = str(path.readlink()).encode() if path.is_symlink() else path.read_bytes()
                    files[name] = {"mode": self.git_modes.get(name, "120000" if path.is_symlink() else "100644"),
                                   "oid": hashlib.sha1(f"blob {len(data)}\0".encode() + data).hexdigest()}
        repositories = [{"path": "qemu", "commit": self.sources["qemu"]["commit"], "tree": "a" * 40}]
        repositories += [{"path": "qemu/" + p["path"], "commit": p["commit"], "tree": "a" * 40}
                         for p in self.sources["submodules"]]
        self.packages["sourceTrees"] = {}
        for repo in repositories:
            prefix = repo["path"] + "/"
            local = {p[len(prefix):]: (r["mode"], r["oid"]) for p, r in files.items()
                     if p.startswith(prefix) and max((x["path"] for x in repositories if p.startswith(x["path"] + "/")), key=len) == repo["path"]}
            repo["gitlinks"] = {p["path"]: p["commit"] for p in self.sources["submodules"]} if repo["path"] == "qemu" else {}
            local.update({p: ("160000", oid) for p, oid in repo["gitlinks"].items()})
            tree = verify.git_tree(local)
            repo.update(tree=tree, indexTree=tree)
            self.packages["sourceTrees"][repo["path"]] = tree
        self.package_lock.write_bytes(encoded(self.packages))
        self.evidence["lockSha256"]["packages"] = hashlib.sha256(self.package_lock.read_bytes()).hexdigest()
        for name, lock in (("sources", self.source_lock), ("packages", self.package_lock)):
            self.put(self.source, f"recipe/{name}.lock.json", lock.read_bytes())
        self.put(self.source, "provenance/git-index.json", encoded({"files": files, "repositories": repositories}))
        recipe.package(self.runtime, self.source, self.output,
                       (self.source_lock, self.package_lock), self.evidence)
        return verify.verify(self.output, self.source_lock, self.package_lock)

    def rewrite(self, filename, mutate):
        path = self.output / filename
        with zipfile.ZipFile(path) as archive:
            entries = [(i, archive.read(i)) for i in archive.infolist()]
        entries = mutate(entries)
        with zipfile.ZipFile(path, "w") as archive:
            for info, data in entries:
                archive.writestr(info, data)
        sums = "".join(f"{hashlib.sha256((self.output / n).read_bytes()).hexdigest()}  {n}\n"
                       for n in verify.ARCHIVES)
        (self.output / "SHA256SUMS").write_text(sums)

    def test_console_only_runtime_and_img_are_required(self):
        self.assertEqual(verify.EXES, {"bin/qemu-system-x86_64.exe", "bin/qemu-img.exe"})
        self.build()

    def test_self_consistent_source_rewrite_cannot_change_external_tree(self):
        self.put(self.source, "qemu/extra.c", b"int original;\n")
        self.build()
        def change(entries):
            contents = {i.filename: data for i, data in entries}
            contents["qemu/extra.c"] = b"int substituted;\n"
            index = json.loads(contents["provenance/git-index.json"])
            value = contents["qemu/extra.c"]
            index["files"]["qemu/extra.c"]["oid"] = hashlib.sha1(f"blob {len(value)}\0".encode() + value).hexdigest()
            root = next(r for r in index["repositories"] if r["path"] == "qemu")
            local = {p[5:]: (r["mode"], r["oid"]) for p, r in index["files"].items()
                     if not p.startswith("qemu/roms/seabios/")}
            local.update({p: ("160000", oid) for p, oid in root["gitlinks"].items()})
            root["indexTree"] = verify.git_tree(local)
            contents["provenance/git-index.json"] = encoded(index)
            manifest = json.loads(contents["provenance/source-manifest.json"])
            for row in manifest["files"]:
                value = contents[row["path"]]
                row.update(bytes=len(value), sha256=hashlib.sha256(value).hexdigest(),
                           gitBlob=hashlib.sha1(f"blob {len(value)}\0".encode() + value).hexdigest())
            contents["provenance/source-manifest.json"] = encoded(manifest)
            return [(i, contents[i.filename]) for i, _ in entries]
        self.rewrite(verify.ARCHIVES[1], change)
        with self.assertRaisesRegex(ValueError, "tree differs from external lock"):
            verify.verify(self.output, self.source_lock, self.package_lock)

    def test_git_tree_format_oracle(self):
        # Independently measured with git mktree --missing, including directory sorting.
        rows = {"foo/x": ("100644", "1" * 40), "foo.c": ("100644", "2" * 40), "bar": ("160000", "3" * 40)}
        self.assertEqual(verify.git_tree(rows), "b9a63093307d01fda5907994ecd3faac9a5a3bfa")

    def test_round_trip_and_repeat_bytes(self):
        result = self.build()
        self.assertFalse(result["publicationReady"])
        self.assertEqual(result["qualification"], "build-only; no VM boot")
        first = [(self.output / name).read_bytes() for name in verify.ARCHIVES]
        self.output = self.root / "again"
        self.build()
        self.assertEqual(first, [(self.output / name).read_bytes() for name in verify.ARCHIVES])

    def test_missing_corrupt_extra_and_duplicate_member(self):
        self.build()
        for action in (lambda es: es[1:], lambda es: [(es[0][0], b"bad"), *es[1:]],
                       lambda es: [*es, ("extra", b"x")], lambda es: [*es, es[0]]):
            with self.subTest(action=action), self.assertRaises(ValueError):
                self.rewrite(verify.ARCHIVES[0], action)
                verify.verify(self.output, self.source_lock, self.package_lock)
            self.output = self.root / (self.output.name + "x")
            self.build()

    def test_external_lock_mismatch(self):
        trusted = self.root / "trusted.json"
        trusted.write_bytes(self.source_lock.read_bytes())
        self.sources["sourceDateEpoch"] += 2
        self.source_lock.write_bytes(encoded(self.sources))
        self.evidence["lockSha256"]["sources"] = hashlib.sha256(self.source_lock.read_bytes()).hexdigest()
        self.build()  # Both manifests, the report and sums agree with the substituted lock.
        with self.assertRaisesRegex(ValueError, "lock"):
            verify.verify(self.output, trusted, self.package_lock)

    def test_wrong_pe_architecture_and_missing_notices(self):
        self.put(self.runtime, self.exes[0], pe(0x14c))
        with self.assertRaisesRegex(ValueError, "AMD64|PE"):
            self.build()
        self.put(self.runtime, self.exes[0], pe())
        (self.runtime / "LICENSES/msys2/lib/COPYING").unlink()
        with self.assertRaisesRegex(ValueError, "notice|license"):
            self.build()

    def test_missing_sources_firmware_and_import_evidence(self):
        for key in ("imports", "firmware", "installed", "features", "tools"):
            evidence = self.evidence.copy()
            del evidence[key]
            with self.subTest(key=key), self.assertRaises(ValueError):
                recipe.package(self.runtime, self.source, self.output, (self.source_lock, self.package_lock), evidence)
        (self.source / "inputs/sources/lib.src.tar.zst").unlink()
        with self.assertRaisesRegex(ValueError, "source|missing"):
            self.build()

    def test_rebuild_recipe_and_reference_notice_required(self):
        recipe_file = self.source / "recipe/build.sh"
        recipe_file.unlink()
        with self.assertRaisesRegex(ValueError, "recipe|rebuild"):
            self.build()
        recipe_file.write_bytes(b"#!/bin/bash\n")
        (self.runtime / "LICENSES/reference/LICENSE").unlink()
        with self.assertRaisesRegex(ValueError, "reference|license"):
            self.build()

    def test_owned_policy_patch_is_authenticated(self):
        self.put(self.source, "recipe/disable-dsound.patch", b"corrupt")
        with self.assertRaisesRegex(ValueError, "policy patch"):
            self.build()
        self.assertFalse(self.output.exists())

    def test_qemu_license_bytes_must_match_source(self):
        for name in ("COPYING", "COPYING.LIB", "LICENSE"):
            path = self.runtime / "LICENSES/qemu" / name
            original = path.read_bytes()
            for value in (b"", b"substituted notice"):
                path.write_bytes(value)
                with self.subTest(name=name, value=value), self.assertRaisesRegex(ValueError, "QEMU.*license"):
                    self.build()
                self.assertFalse(self.output.exists())
                self.assertFalse(list(self.root.glob(".bridge-runtime-*")))
            path.write_bytes(original)

    def test_raw_zip_name_is_validated_before_python_normalization(self):
        self.put(self.runtime, "extraXtrailer", b"opaque extra")
        self.build()
        def change(entries):
            result = []
            for info, data in entries:
                if info.filename.endswith("runtime-manifest.json"):
                    manifest = json.loads(data)
                    next(r for r in manifest["files"] if r["path"] == "extraXtrailer")["path"] = "extra"
                    data = encoded(manifest)
                result.append((info, data))
            return result
        self.rewrite(verify.ARCHIVES[0], change)
        path = self.output / verify.ARCHIVES[0]
        raw = path.read_bytes()
        self.assertEqual(raw.count(b"extraXtrailer"), 2)
        path.write_bytes(raw.replace(b"extraXtrailer", b"extra\0trailer"))
        (self.output / "SHA256SUMS").write_text("".join(f"{verify.file_hash(self.output / n)}  {n}\n" for n in verify.ARCHIVES))
        with self.assertRaisesRegex(ValueError, "unsafe archive path"):
            verify.verify(self.output, self.source_lock, self.package_lock)

    def test_notice_bytes_and_firmware_owner(self):
        notice = self.runtime / "LICENSES/msys2/lib/COPYING"
        notice.write_bytes(b"not a license")
        with self.assertRaisesRegex(ValueError, "notice|license"):
            self.build()
        notice.write_bytes(b"MIT notice")
        self.packages["firmware"][0]["source"] = "firmware/missing"
        self.package_lock.write_bytes(encoded(self.packages))
        self.evidence["lockSha256"]["packages"] = hashlib.sha256(self.package_lock.read_bytes()).hexdigest()
        with self.assertRaisesRegex(ValueError, "firmware|source"):
            self.build()

    def test_unknown_dll_package_and_inventory(self):
        for change in (lambda e: e["imports"][self.exes[0]][1].update(owner="unknown"),
                       lambda e: e["imports"][self.exes[0]][1].update(path="bin/missing.dll"),
                       lambda e: e["installed"].append({"name": "unknown", "version": "1"}),
                       lambda e: e["features"].update(tcg=True)):
            evidence = copy.deepcopy(self.evidence)
            change(evidence)
            with self.subTest(change=change), self.assertRaises(ValueError):
                recipe.package(self.runtime, self.source, self.output, (self.source_lock, self.package_lock), evidence)

    def test_windows_path_hazards(self):
        for name in ("../x", "/x", "C:/x", "\\\\server\\x", "a/b:c", "a/NUL.txt", "a/COM1", "a/CON .txt", "a/CONIN$", "a/COM¹.txt", "a/b.",
                     "a/b ", "a\x00b", "a\\b", "a//b", "a/./b"):
            with self.subTest(name=name), self.assertRaises(ValueError):
                verify.safe_path(name)
        self.build()
        alias = zipfile.ZipInfo("BIN/QEMU-IMG.EXE")
        alias.external_attr = 0o100644 << 16
        self.rewrite(verify.ARCHIVES[0], lambda entries: [*entries, (alias, pe())])
        with self.assertRaisesRegex(ValueError, "collision|duplicate"):
            verify.verify(self.output, self.source_lock, self.package_lock)

    def test_source_links_safe_escape_dangling_cycle(self):
        link = self.source / "qemu/link"
        try:
            link.symlink_to("COPYING")
        except OSError as error:
            self.skipTest(str(error))
        self.build()
        for target in ("../../outside", "missing", "link"):
            link.unlink()
            link.symlink_to(target)
            self.output = self.root / (self.output.name + "x")
            with self.subTest(target=target), self.assertRaisesRegex(ValueError, "link"):
                self.build()

    def test_limits_and_no_success_on_failure(self):
        for key, value in (("MAX_ENTRIES", 1), ("MAX_RUNTIME", 20), ("MAX_SOURCE", 20)):
            self.output = self.root / (self.output.name + "x")
            with self.subTest(key=key), patch.object(verify, key, value), self.assertRaises(ValueError):
                self.build()
            self.assertFalse((self.output / "verification.json").exists())

    def test_git_blob_integrity_and_mode_placeholders(self):
        self.put(self.source, "qemu/fork.rs", b"fn main() {\n    println!(\"source, not a link\");\n}\n")
        self.git_modes["qemu/fork.rs"] = "120000"
        self.build()
        def change(entries):
            value = b"different source"
            result = []
            for info, data in entries:
                if info.filename == "qemu/COPYING":
                    data = value
                elif info.filename == "provenance/source-manifest.json":
                    manifest = json.loads(data)
                    row = next(r for r in manifest["files"] if r["path"] == "qemu/COPYING")
                    row.update(bytes=len(value), sha256=hashlib.sha256(value).hexdigest(),
                               gitBlob=hashlib.sha1(f"blob {len(value)}\0".encode() + value).hexdigest())
                    data = encoded(manifest)
                result.append((info, data))
            return result
        self.rewrite(verify.ARCHIVES[1], change)
        with self.assertRaisesRegex(ValueError, "Git|git"):
            verify.verify(self.output, self.source_lock, self.package_lock)

    def test_duplicate_manifest_keys(self):
        self.build()
        def change(entries):
            return [(info, b'{"files":[],"files":[]}' if info.filename.endswith("runtime-manifest.json") else data)
                    for info, data in entries]
        self.rewrite(verify.ARCHIVES[0], change)
        with self.assertRaisesRegex(ValueError, "duplicate"):
            verify.verify(self.output, self.source_lock, self.package_lock)


if __name__ == "__main__":
    unittest.main()
