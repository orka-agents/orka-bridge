"""Synthetic non-bootable payloads exercise packaging, never VM readiness."""
import copy
import hashlib
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from images import package
from images.guest.tests.fixtures import lock


def sha(data):
    return hashlib.sha256(data).hexdigest()


def metadata(root, platform="windows") -> dict:
    value = lock(root)
    if platform == "macos":
        value["architecture"] = "arm64"
        value["nodeLocalRef"] = value["nodeLocalRef"].replace("amd64", "arm64")
    return {"imageId": "synthetic-not-bootable", "buildSourceCommit": "a" * 40,
            "platform": platform, "architecture": value["architecture"],
            "format": "raw" if platform == "macos" else "qcow2",
            "minimumHostVersion": "13.0" if platform == "macos" else "11.0",
            "minimumAppVersion": "0.1.0", "stateCompatibilityId": "fixture-state-v1",
            "transportProtocolMajor": 1, "componentLock": value}


def boot_inputs(root, meta, payload):
    """Complete guest receipt shape; synthetic bytes are never boot evidence."""
    root.mkdir(exist_ok=True)
    arch = meta["architecture"]
    kernel = bytearray(4096)
    if arch == "amd64":
        kernel[0x1fe:0x200], kernel[0x202:0x206] = b"\x55\xaa", b"HdrS"
    else:
        kernel[0x38:0x3c] = b"ARM\x64"
    files = {"kernel": root / "os.kernel", "initrd": root / "os.initrd",
             "build_receipt": root / "build.json", "boot_receipt": root / "boot-assets.json"}
    files["kernel"].write_bytes(kernel)
    files["initrd"].write_bytes(b"\x1f\x8bsynthetic initramfs")
    lock_value = meta["componentLock"]
    for name in ("guestfish", "unmkinitramfs", "readelf", "dpkg-query", "bridge-guest"):
        if not any(t["name"] == name for t in lock_value["tools"]):
            lock_value["tools"].append(dict(name=name, version="tool 1.2\nCopyright fixture", path=name, sha256=sha(name.encode())))
    for name in ("bridge-guest", "bridge-guest-provenance"):
        record = next((t for t in lock_value["tools"] if t["name"] == name), None)
        if record is None:
            record = dict(name=name, path=name, sha256=sha(name.encode()))
            lock_value["tools"].append(record)
        record["version"] = meta["buildSourceCommit"]
    inputs = sha(json.dumps(lock_value, sort_keys=True, separators=(",", ":")).encode())
    recipe = {"/etc/initramfs-tools/hooks/bridge-state": "1" * 64,
              "/etc/initramfs-tools/scripts/local-bottom/bridge-state": "2" * 64}
    build = dict(schemaVersion="bridge-guest-build/v1", architecture=arch,
                 contract={}, inputSha256=inputs, builderTools=[],
                 runtime=dict(schemaVersion="bridge-guest-binary/v1", sourceCommit=meta["buildSourceCommit"],
                              binarySha256=sha(b"bridge-guest"), binaryBytes=12,
                              target="aarch64-unknown-linux-musl" if arch == "arm64" else "x86_64-unknown-linux-musl",
                              contractVersion="H1/v1", protocolMajor=1,
                              publicationReady=False, qualification={}, toolchain="1.98.1", rustc="rustc 1.98.1",
                              rustcBinarySha256="6" * 64, cargoLockSha256="7" * 64, elf={}),
                 recipeFiles=recipe, payload="os.raw" if arch == "arm64" else "os.qcow2",
                 sha256=sha(payload), fileBytes=len(payload), allocatedBytes=4096,
                 componentCacheVerified=False, bootVerified=False, publicationReady=False)
    files["build_receipt"].write_bytes(package.canonical(build))
    uuid = "11111111-2222-4333-8444-555555555555"
    console = "hvc0" if arch == "arm64" else "ttyS0,115200n8"
    guest_dir = Path(package.guest.__file__).parent
    receipt = dict(schemaVersion="bridge-guest-boot-assets/v1", architecture=arch,
                   diskSha256=sha(payload), buildReceiptSha256=sha(files["build_receipt"].read_bytes()), inputSha256=inputs,
                   kernel={"name": "os.kernel", "bytes": len(kernel), "sha256": sha(kernel),
                           "format": "linux-arm64-image" if arch == "arm64" else "linux-bzimage"},
                   initrd={"name": "os.initrd", "bytes": files["initrd"].stat().st_size,
                           "sha256": sha(files["initrd"].read_bytes()), "format": "initramfs", "compression": "gzip"},
                   rootSelector={"kind": "filesystem-uuid", "value": uuid}, kernelRelease="6.1.0-fixture-" + arch,
                   commandLine="root=UUID=" + uuid + " ro console=" + console + " noresume",
                   requiredDrivers=[{"name": n, "builtIn": True} for n in
                                    ("virtio_pci", "virtio_blk", "virtio_console", "virtio_rng", "virtio_net", "ext4")],
                   kernelConfigSha256="3" * 64, modulesSha256="4" * 64, endpointSha256=sha(b"bridge-guest"),
                   earlyHookSha256="2" * 64, buildHookSha256="1" * 64,
                   extractorSha256=sha((guest_dir / "boot_assets.py").read_bytes()),
                   inspectorSha256=sha((guest_dir / "boot_inspect.py").read_bytes()),
                   extractionTools=[{"name": n, "version": "tool 1.2\nCopyright fixture", "executableSha256": sha(n.encode())}
                                    for n in ("guestfish", "unmkinitramfs", "readelf", "dpkg-query")],
                   initrdTools=[dict(initrdPath=n, guestCanonicalPath=n, bytes=12, guestSha256="5" * 64,
                                     initrdSha256="5" * 64, initrdType="regular", dependencies=[]) for n in
                                ("/sbin/blkid", "/sbin/blockdev", "/sbin/mkfs.ext4", "/bin/mount", "/usr/bin/findmnt")],
                   bootVerified=False, publicationReady=False)
    files["boot_receipt"].write_bytes(package.canonical(receipt))
    return files


class BoundedStream(io.BytesIO):
    def read(self, size=-1):
        assert 0 < size <= 1024 * 1024, size
        return super().read(size)


class PackageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.meta = metadata(self.root)
        self.payload = b"synthetic disk, not a VM\n" * 73

    def make(self, name="package", chunk=17, payload=None):
        out = self.root / name
        inputs = boot_inputs(self.root / "boot", self.meta, self.payload)
        result = package.create(BoundedStream(self.payload if payload is None else payload), self.meta, out, chunk, **inputs)
        return out, result

    def save(self, out, value):
        (out / "manifest.json").write_bytes(package.canonical(value))

    def test_v2_requires_direct_pair_and_separate_host_launch(self):
        _, manifest = self.make()
        self.assertEqual(manifest["schemaVersion"], "bridge-image/v2")
        self.assertEqual(manifest["defaultLaunch"], "linux-direct")
        self.assertEqual(manifest["layout"]["osDisk"]["boot"], "uefi")
        self.assertIn("directBoot", manifest)

    def test_v1_is_explicitly_unsupported_without_upgrade(self):
        _, manifest = self.make()
        manifest["schemaVersion"] = "bridge-image/v1"
        with self.assertRaisesRegex(ValueError, "unsupported manifest schema"):
            package.validate_manifest(manifest)

    def test_roundtrip_deterministic_streaming_and_measured_bytes(self):
        first, manifest = self.make()
        second, other = self.make("again")
        self.assertEqual(manifest, other)
        self.assertEqual({p.name: p.read_bytes() for p in first.iterdir()},
                         {p.name: p.read_bytes() for p in second.iterdir()})
        restored = self.root / "restored.disk"
        self.assertEqual(package.verify(first / "manifest.json", out=restored), manifest)
        self.assertEqual(restored.read_bytes(), self.payload)
        self.assertEqual(manifest["artifact"]["uncompressedBytes"], len(self.payload))
        self.assertEqual(manifest["artifact"]["uncompressedSha256"], sha(self.payload))
        self.assertFalse(manifest["publicationReady"])
        self.assertNotEqual(manifest["artifact"]["uncompressedBytes"], manifest["layout"]["osDisk"]["virtualCapacityBytes"])

    def test_boot_pair_and_provenance_match_task2_inputs(self):
        for platform, launch, fmt, console in (("windows", "linux-direct", "linux-bzimage", "ttyS0,115200n8"),
                                               ("macos", "uefi", "linux-arm64-image", "hvc0")):
            self.meta = metadata(self.root, platform)
            out, manifest = self.make(platform)
            direct = manifest["directBoot"]
            self.assertEqual(manifest["defaultLaunch"], launch)
            self.assertEqual(direct["kernel"]["format"], fmt)
            self.assertEqual(direct["commandLine"], "root=UUID=11111111-2222-4333-8444-555555555555 ro console=" + console + " noresume")
            self.assertEqual(direct["provenance"]["diskSha256"], sha(self.payload))
            self.assertNotEqual(direct["provenance"]["inputSha256"], manifest["components"]["canonicalLockSha256"])
            self.assertEqual(direct["provenance"]["extractionTools"][0]["version"], "tool 1.2\nCopyright fixture")
            self.assertEqual({p.name for p in out.iterdir()}, {"manifest.json", direct["kernel"]["name"], direct["initrd"]["name"],
                             *(c["name"] for c in manifest["artifact"]["chunks"])})
            package.verify(out / "manifest.json")

    def test_consumes_actual_task2_extractor_receipt_not_an_alternate_format(self):
        from images.guest.tests.test_boot_assets import BootAssetTests
        producer = BootAssetTests()
        producer.setUp()
        self.addCleanup(producer.doCleanups)
        producer.build["runtime"] = json.loads((producer.root / "provenance.json").read_bytes())
        receipt = producer.extract()  # Real extractor; only guest/tool I/O is synthetic.
        meta = dict(self.meta, componentLock=producer.value)
        output = self.root / "task2-package"
        exported = producer.root / "assets"
        manifest = package.create(BoundedStream(producer.image.read_bytes()), meta, output,
                                  build_receipt=producer.receipt, boot_receipt=exported / "boot-assets.json",
                                  kernel=exported / "os.kernel", initrd=exported / "os.initrd")
        self.assertEqual(manifest["directBoot"]["provenance"]["inputSha256"], receipt["inputSha256"])
        self.assertEqual(manifest["directBoot"]["provenance"]["inspectorSha256"], receipt["inspectorSha256"])
        package.verify(output / "manifest.json")

    def test_bad_receipts_and_inputs_reject_before_package_writes(self):
        changes = [("architecture", "arm64"), ("kernelRelease", "6.1-other-arm64"),
                   ("diskSha256", "f" * 64), ("inputSha256", "f" * 64),
                   ("buildReceiptSha256", "f" * 64), ("extractorSha256", "f" * 64),
                   ("inspectorSha256", "f" * 64), ("endpointSha256", "f" * 64),
                   ("earlyHookSha256", "f" * 64), ("buildHookSha256", "f" * 64),
                   ("commandLine", "root=/dev/sda3 ro"), ("rootSelector", {"kind": "filesystem-uuid", "value": "bad"}),
                   ("requiredDrivers", []), ("extractionTools", []), ("derivation", {})]
        for field, value in changes:
            inputs = boot_inputs(self.root / "boot", self.meta, self.payload)
            receipt = json.loads(inputs["boot_receipt"].read_bytes()); receipt[field] = value
            inputs["boot_receipt"].write_bytes(package.canonical(receipt))
            with self.subTest(field=field), self.assertRaises((ValueError, OSError)):
                package.create(BoundedStream(self.payload), self.meta, self.root / "bad", **inputs)
            self.assertFalse((self.root / "bad").exists())

    def test_foreign_runtime_source_refused_before_writes(self):
        inputs = boot_inputs(self.root / "boot", self.meta, self.payload)
        build = json.loads(inputs["build_receipt"].read_bytes())
        build["runtime"]["sourceCommit"] = "b" * 40
        inputs["build_receipt"].write_bytes(package.canonical(build))
        receipt = json.loads(inputs["boot_receipt"].read_bytes())
        receipt["buildReceiptSha256"] = sha(inputs["build_receipt"].read_bytes())
        inputs["boot_receipt"].write_bytes(package.canonical(receipt))
        with self.assertRaisesRegex(ValueError, "runtime"):
            package.create(BoundedStream(self.payload), self.meta, self.root / "bad", **inputs)
        self.assertFalse((self.root / "bad").exists())

    def test_boot_files_missing_linked_swapped_changed_or_truncated_refused(self):
        for role in ("kernel", "initrd"):
            for kind in ("missing", "symlink", "swapped", "changed", "truncated"):
                out, manifest = self.make(role + kind)
                path = out / manifest["directBoot"][role]["name"]
                original = path.read_bytes()
                if kind == "missing": path.unlink()
                if kind == "symlink":
                    path.unlink(); path.symlink_to(self.root / "boot" / ("os." + role))
                if kind == "swapped": path.write_bytes((out / manifest["directBoot"]["initrd" if role == "kernel" else "kernel"]["name"]).read_bytes())
                if kind == "changed": path.write_bytes(b"!" * len(original))
                if kind == "truncated": path.write_bytes(original[:-1])
                with self.subTest(role=role, kind=kind), self.assertRaises((ValueError, OSError)):
                    package.verify(out / "manifest.json", out=self.root / "no-restored")
                self.assertFalse((self.root / "no-restored.partial").exists())

    def test_source_boot_files_and_receipts_refuse_before_output(self):
        for role in ("kernel", "initrd", "build_receipt", "boot_receipt"):
            for kind in ("missing", "symlink", "changed", "truncated"):
                inputs = boot_inputs(self.root / "boot", self.meta, self.payload)
                path = inputs[role]; data = path.read_bytes()
                if kind == "missing": path.unlink()
                if kind == "symlink":
                    copy = self.root / "copy"; copy.write_bytes(data)
                    path.unlink(); path.symlink_to(copy)
                if kind == "changed": path.write_bytes(b"!" * len(data))
                if kind == "truncated": path.write_bytes(data[:len(data) // 2])
                with self.subTest(role=role, kind=kind), self.assertRaises((ValueError, OSError)):
                    package.create(BoundedStream(self.payload), self.meta, self.root / "bad", **inputs)
                self.assertFalse((self.root / "bad").exists())
                if path.is_symlink(): path.unlink()

    def test_tool_versions_are_bounded_text_but_other_versions_keep_policy(self):
        for version in ("", " " * 20, "x" * 4097, "tool\x00bad"):
            meta = copy.deepcopy(self.meta)
            inputs = boot_inputs(self.root / "boot", meta, self.payload)
            meta["componentLock"]["tools"][1]["version"] = version
            with self.subTest(version=version[:20]), self.assertRaises(ValueError):
                package.create(BoundedStream(self.payload), meta, self.root / "bad", **inputs)
            self.assertFalse((self.root / "bad").exists())

    def test_v2_metadata_and_format_mismatch_refused(self):
        mutations = [lambda m: m.pop("directBoot"), lambda m: m.update(defaultLaunch="uefi"),
                     lambda m: m["directBoot"]["kernel"].update(format="linux-arm64-image"),
                     lambda m: m["directBoot"]["initrd"].update(compression="xz"),
                     lambda m: m["directBoot"].update(commandLine="root=UUID=other ro"),
                     lambda m: m["directBoot"]["provenance"].update(diskSha256="f" * 64),
                     lambda m: m["directBoot"]["kernel"].update(name="../kernel"),
                     lambda m: m["layout"]["osDisk"].update(boot="linux-direct")]
        for i, change in enumerate(mutations):
            out, manifest = self.make("bad-v2-" + str(i)); change(manifest); self.save(out, manifest)
            with self.subTest(i=i), self.assertRaises(ValueError):
                package.verify(out / "manifest.json")

    def test_exact_boundary_and_short_final_have_no_empty_chunk(self):
        _, initial = self.make(chunk=package.MAX_CHUNK)
        size = initial["artifact"]["compressedBytes"]
        exact, a = self.make("exact", size)
        self.assertEqual([x["bytes"] for x in a["artifact"]["chunks"]], [size])
        short, b = self.make("short", size - 1)
        self.assertEqual([x["bytes"] for x in b["artifact"]["chunks"]], [size - 1, 1])
        package.verify(exact / "manifest.json")
        package.verify(short / "manifest.json")

    def test_zero_input_and_invalid_chunk_sizes_fail_before_output(self):
        for chunk in [0, -1, True, 1.5, package.MAX_CHUNK + 1]:
            with self.subTest(chunk=chunk), self.assertRaises(ValueError):
                self.make(str(chunk), chunk)
            self.assertFalse((self.root / str(chunk)).exists())
        with self.assertRaises(ValueError):
            self.make("empty", payload=b"")
        self.assertFalse((self.root / "empty").exists())

    def test_roles_capacities_and_target_mapping(self):
        for platform, fmt, arch in [("windows", "qcow2", "amd64"), ("macos", "raw", "arm64")]:
            self.meta = metadata(self.root, platform)
            _, m = self.make(platform)
            self.assertEqual(m["target"]["architecture"], arch)
            self.assertEqual(m["layout"]["osDisk"]["format"], fmt)
            state = m["layout"]["stateDisk"]
            self.assertEqual((state["role"], state["operation"], state["format"]), ("state-disk", "create-empty", fmt))
            self.assertEqual(state["virtualCapacityBytes"], 34359738368)
            self.assertEqual(m["layout"]["osDisk"]["virtualCapacityBytes"], 17179869184)
            self.assertEqual(m["layout"]["devices"]["controlPort"], "ai.orka.bridge.control")
            self.assertEqual(m["artifact"]["role"], "os-disk")

    def test_component_identities_are_distinct_and_no_private_paths_projected(self):
        _, m = self.make()
        c = m["components"]
        self.assertIsNone(c["kmxProfileSha256"])
        self.assertEqual(c["kubernetes"]["provenance"], "policy-declared")
        self.assertEqual(c["versionPolicySha256"], sha(Path(package.guest.__file__).with_name("versions.json").read_bytes()))
        self.assertNotEqual(c["inputIdentity"], c["canonicalLockSha256"])
        text = json.dumps(m)
        for unwanted in ["chartPath", "archiveSha256", "base.qcow2", "installationUuid", "bootNonce", "credentialId"]:
            self.assertNotIn(unwanted, text)
        self.assertIn("kind", [t["name"] for t in c["tools"]])
        self.assertEqual([t["name"] for t in c["tools"]], sorted(t["name"] for t in c["tools"]))

    def test_bad_metadata_is_rejected_without_writing(self):
        for field, value in [("architecture", "arm64"), ("format", "raw"), ("platform", "linux"),
                             ("imageId", "../../escape"), ("imageId", "fixture..v1"), ("buildSourceCommit", "main"),
                             ("transportProtocolMajor", 2), ("transportProtocolMajor", True),
                             ("minimumAppVersion", "latest"), ("stateCompatibilityId", "")]:
            bad = copy.deepcopy(self.meta); bad[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                package.create(io.BytesIO(self.payload), bad, self.root / "bad",
                               **boot_inputs(self.root / "boot", self.meta, self.payload))
            self.assertFalse((self.root / "bad").exists())
        for key in ["statePayload", "token", "localPath", "installationUuid", "publicationReady"]:
            bad = dict(self.meta, **{key: "PRIVATE_MARKER"})
            with self.subTest(key=key), self.assertRaises(ValueError):
                package.validate_metadata(bad)

    def test_corrupt_truncated_missing_extra_and_link_chunks(self):
        for kind in ["corrupt", "truncated", "missing", "extra", "symlink"]:
            out, m = self.make(kind)
            chunk = out / m["artifact"]["chunks"][0]["name"]
            if kind == "corrupt": chunk.write_bytes(b"!" * chunk.stat().st_size)
            if kind == "truncated": chunk.write_bytes(chunk.read_bytes()[:-1])
            if kind == "missing": chunk.unlink()
            if kind == "extra": (out / "unexpected").write_text("not selected")
            if kind == "symlink":
                chunk.unlink(); chunk.symlink_to(self.root / "kind")
            with self.subTest(kind=kind), self.assertRaises((ValueError, OSError)):
                package.verify(out / "manifest.json")

    def test_manifest_boundary_schema_order_duplicates_and_layout(self):
        changes = [lambda m: m.update(schemaVersion="bridge-image/v1"),
                   lambda m: m.update(publicationReady=True),
                   lambda m: m["artifact"]["chunks"].reverse(),
                   lambda m: m["artifact"]["chunks"].append(m["artifact"]["chunks"][0]),
                   lambda m: m["artifact"]["chunks"][0].update(name="../escape"),
                   lambda m: m["layout"]["stateDisk"].update(operation="reuse-initialized"),
                   lambda m: m["layout"]["osDisk"].update(virtualCapacityBytes=1),
                   lambda m: m["components"].update(token="PRIVATE_MARKER"),
                   lambda m: m["layout"]["transport"].update(protocolMajor=True)]
        for i, change in enumerate(changes):
            out, m = self.make("manifest" + str(i)); change(m); self.save(out, m)
            with self.subTest(i=i), self.assertRaises(ValueError): package.verify(out / "manifest.json")

    def test_expected_manifest_hash_is_an_external_anchor(self):
        out, _ = self.make()
        path = out / "manifest.json"
        package.verify(path, manifest_sha256=sha(path.read_bytes()))
        with self.assertRaisesRegex(ValueError, "manifest"):
            package.verify(path, manifest_sha256="0" * 64)

    def test_gzip_corruption_is_detected_even_with_updated_chunk_hashes(self):
        out, m = self.make(chunk=package.MAX_CHUNK)
        entry = m["artifact"]["chunks"][0]; path = out / entry["name"]
        data = bytearray(path.read_bytes()); data[-8] ^= 1; path.write_bytes(data)
        entry["sha256"] = m["artifact"]["compressedSha256"] = sha(data)
        self.save(out, m)
        with self.assertRaises(ValueError): package.verify(out / "manifest.json")

    def test_output_is_not_overwritten_and_failed_write_never_finishes_manifest(self):
        out, _ = self.make()
        before = (out / "manifest.json").read_bytes()
        with self.assertRaises((ValueError, OSError)): self.make()
        self.assertEqual((out / "manifest.json").read_bytes(), before)
        original = package.canonical
        def fail_final(value):
            if value.get("schemaVersion") == "bridge-image/v2":
                raise OSError("injected manifest write failure")
            return original(value)
        with patch.object(package, "canonical", side_effect=fail_final):
            with self.assertRaises(OSError): self.make("interrupted")
        self.assertTrue(list((self.root / "interrupted").glob("*.part-*")))
        self.assertFalse((self.root / "interrupted" / "manifest.json").exists())
        with self.assertRaises((ValueError, OSError)): package.verify(out / "manifest.json", out=out / "manifest.json")

    def test_reconstruction_cannot_invalidate_its_own_package_directory(self):
        out, _ = self.make()
        original_names = {p.name for p in out.iterdir()}
        with self.assertRaisesRegex(ValueError, "outside package"):
            package.verify(out / "manifest.json", out=out / "restored.disk")
        self.assertEqual({p.name for p in out.iterdir()}, original_names)
        package.verify(out / "manifest.json")

    def test_verified_output_never_replaces_a_concurrently_created_file(self):
        out, _ = self.make()
        target = self.root / "raced.disk"
        original = package.ChunkReader.readinto
        def race(reader, buffer):
            target.write_bytes(b"caller-owned")
            return original(reader, buffer)
        with patch.object(package.ChunkReader, "readinto", race), self.assertRaises(FileExistsError):
            package.verify(out / "manifest.json", out=target)
        self.assertEqual(target.read_bytes(), b"caller-owned")

    def test_external_hash_binds_the_bytes_actually_parsed(self):
        out, m = self.make()
        path = out / "manifest.json"
        replacement = dict(m, stateCompatibilityId="another-state")
        expected = sha(package.canonical(replacement))
        original = package.read_json
        def swap(path, *args, **kwargs):
            value = original(path, *args, **kwargs)
            path.write_bytes(package.canonical(replacement))
            return value
        with patch.object(package, "read_json", swap), self.assertRaisesRegex(ValueError, "manifest"):
            package.verify(path, manifest_sha256=expected)

    def test_json_duplicate_keys_and_large_metadata_are_rejected(self):
        p = self.root / "metadata.json"
        p.write_text('{"imageId":"a","imageId":"b"}')
        with self.assertRaises(ValueError): package.read_json(p)
        p.write_bytes(b" " * (package.MAX_JSON + 1))
        with self.assertRaises(ValueError): package.read_json(p)

    def test_schema_has_the_exact_top_level_contract_and_provisional_notice(self):
        out, m = self.make()
        schema = json.loads(Path(package.__file__).with_name("manifest.schema.json").read_text())
        self.assertEqual(set(schema["required"]), set(m))
        self.assertEqual(set(schema["properties"]), set(m))
        self.assertFalse(schema["additionalProperties"])
        self.assertIn("provisional", schema["$comment"].lower())
        self.assertEqual(schema["properties"]["schemaVersion"]["const"], m["schemaVersion"])
        layouts = schema["properties"]["layout"]["properties"]
        state = layouts["stateDisk"]
        self.assertEqual(set(state["required"]), set(m["layout"]["stateDisk"]))
        for field in ("role", "operation", "virtualCapacityBytes", "formatPolicy", "blockSerial"):
            self.assertEqual(state["properties"][field]["const"], m["layout"]["stateDisk"][field])
        self.assertEqual(layouts["osDisk"]["properties"]["virtualCapacityBytes"]["const"], 17179869184)
        package.verify(out / "manifest.json")

    def test_root_package_import_does_not_require_guest_pythonpath(self):
        environment = dict(os.environ, PYTHONPATH="")
        result = subprocess.run([sys.executable, "-c", "from images import package; print(package.ROOT[-1])"],
                                cwd=Path(package.__file__).resolve().parents[1], env=environment,
                                capture_output=True, text=True, check=False)
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual("directBoot", result.stdout.strip())

    def test_actual_cli_packages_and_verifies_without_network_or_traceback(self):
        src = self.root / "fixture.disk"; src.write_bytes(self.payload)
        meta = self.root / "metadata.json"; meta.write_text(json.dumps(self.meta))
        script = str(Path(package.__file__))
        inputs = boot_inputs(self.root / "boot", self.meta, self.payload)
        meta.write_text(json.dumps(self.meta))
        flags = [x for k, v in inputs.items() for x in ("--" + k.replace("_", "-"), str(v))]
        a = subprocess.run([sys.executable, script, *flags, "--input", str(src), "--metadata", str(meta),
                            "--out", str(self.root / "cli"), "--chunk-bytes", "17"], capture_output=True, text=True)
        self.assertEqual(a.returncode, 0, a.stderr)
        b = subprocess.run([sys.executable, script, "--verify", str(self.root / "cli/manifest.json")], capture_output=True, text=True)
        self.assertEqual(b.returncode, 0, b.stderr)
        self.assertIn("not boot", b.stdout)
        bad = subprocess.run([sys.executable, script, "--verify", str(self.root / "absent.json")], capture_output=True, text=True)
        self.assertEqual(bad.returncode, 2)
        self.assertNotIn("Traceback", bad.stderr)
