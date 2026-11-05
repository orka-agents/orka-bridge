"""Synthetic filesystem/process boundaries; no guest code or VM is executed."""
import importlib
import importlib.util
import io
import json
import os
import shutil
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from .fixtures import lock, sha
from .test_runtime import runtime_inputs

TOOLS = ("/sbin/blkid", "/sbin/blockdev", "/sbin/mkfs.ext4", "/bin/mount", "/usr/bin/findmnt")
CONFIG = ("VIRTIO_PCI", "VIRTIO_BLK", "VIRTIO_CONSOLE", "HW_RANDOM_VIRTIO", "VIRTIO_NET", "EXT4_FS")


class BootAssetTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(importlib.util.find_spec("guest.boot_assets"), "boot-asset exporter missing")
        self.api = importlib.import_module("guest.boot_assets")
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.value = lock(self.root)
        runtime_inputs(self.root, self.value)
        self.release = "6.1.0-fixture-amd64"
        self.uuid = "11111111-2222-4333-8444-555555555555"
        self.files = {}
        kernel = bytearray(4096)
        kernel[0x1fe:0x200] = b"\x55\xaa"
        kernel[0x202:0x206] = b"HdrS"
        self.files["/boot/vmlinuz-" + self.release] = bytes(kernel)
        self.files["/boot/initrd.img-" + self.release] = b"\x1f\x8bsynthetic-initrd"
        self.files["/boot/config-" + self.release] = "".join("CONFIG_" + c + "=y\n" for c in CONFIG).encode()
        self.files["/etc/default/grub"] = b"GRUB_DEFAULT=0\n"
        self.files["/boot/grub/grub.cfg"] = ('set default="0"\nmenuentry Linux {\nlinux /boot/vmlinuz-' + self.release + " root=PARTUUID=old ro console=ttyS0,115200n8 noresume\ninitrd /boot/initrd.img-" + self.release + "\n").encode()
        self.files["/boot/grub/grubenv"] = b"# GRUB Environment Block\n"
        self.files["/usr/lib/bridge/bridge-guest"] = (self.root / "bridge-guest").read_bytes()
        self.files["/etc/initramfs-tools/hooks/bridge-state"] = b"build hook"
        self.files["/etc/initramfs-tools/scripts/local-bottom/bridge-state"] = b"early hook"
        self.aliases = {"/vmlinuz": "/boot/vmlinuz-" + self.release}
        self.initrd = self.root / "initrd-fixture"
        (self.initrd / "usr/sbin").mkdir(parents=True)
        (self.initrd / "usr/bin").mkdir()
        (self.initrd / "sbin").symlink_to("usr/sbin")
        (self.initrd / "bin").symlink_to("usr/bin")
        for path in TOOLS:
            canonical = "/usr/sbin/mke2fs" if path.endswith("mkfs.ext4") else "/usr" + path if path.startswith(("/bin/", "/sbin/")) else path
            self.aliases[path] = canonical
            self.files[canonical] = b"ELF tool " + path.encode()
            (self.initrd / path.lstrip("/")).write_bytes(self.files[canonical])
            (self.initrd / path.lstrip("/")).chmod(0o755)
        for path, source in (("/usr/lib/bridge/bridge-guest", "/usr/lib/bridge/bridge-guest"),
                             ("/scripts/local-bottom/bridge-state", "/etc/initramfs-tools/scripts/local-bottom/bridge-state")):
            target = self.initrd / path.lstrip("/")
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(self.files[source])
            target.chmod(0o755)
        (self.initrd / "lib").mkdir()
        (self.initrd / "lib/libfixture.so").write_bytes(b"ELF dependency")
        self.files["/lib/libfixture.so"] = b"ELF dependency"
        self.files["/lib/fixture-loader"] = b"ELF loader"
        loader = self.initrd / "lib/fixture-loader"
        loader.write_bytes(self.files["/lib/fixture-loader"])
        loader.chmod(0o755)
        self.modules = {"modules.dep": b"", "modules.builtin": b""}
        self.image = self.root / "os.qcow2"
        self.image.write_bytes(b"final payload")
        self.receipt = self.root / "build.json"
        self.build: dict = {"schemaVersion": "bridge-guest-build/v1", "architecture": "amd64", "payload": self.image.name,
                      "sha256": sha(self.image.read_bytes()), "fileBytes": self.image.stat().st_size,
                      "inputSha256": sha(self.api.encoded(self.value)), "bootVerified": False, "publicationReady": False,
                      "runtime": {"binarySha256": sha(self.files["/usr/lib/bridge/bridge-guest"])},
                      "recipeFiles": {p: sha(self.files[p]) for p in ("/etc/default/grub", "/etc/initramfs-tools/hooks/bridge-state", "/etc/initramfs-tools/scripts/local-bottom/bridge-state")}}
        self.guest_calls = []
        for name in ("guestfish", "unmkinitramfs", "readelf", "dpkg-query"):
            path = self.root / name
            path.write_bytes(name.encode())
            self.value["tools"].append({"name": name, "version": "fixture-version", "path": name, "sha256": sha(path.read_bytes())})
        self.build["inputSha256"] = sha(self.api.encoded(self.value))

    def guest(self, image, executable):
        self.assertEqual(self.root / "guestfish", executable)
        self.assertNotEqual(self.image, image, "inspection must use an owned payload snapshot")
        self.assertEqual(self.build["sha256"], sha(image.read_bytes()))
        return self

    def call(self, command, *args):
        self.guest_calls.append((command, *map(str, args)))
        if command == "inspect-get-roots":
            return "/dev/sda3"
        if command == "vfs-uuid":
            return self.uuid
        if command == "realpath":
            return self.aliases.get(str(args[0]), str(args[0]))
        if command == "cat":
            return self.files[str(args[0])].decode().strip()
        if command == "checksum":
            self.assertEqual("sha256", args[0])
            return sha(self.files[str(args[1])])
        if command == "is-symlink":
            return str(str(args[0]) in self.aliases).lower()
        if command == "is-file":
            return str(str(args[0]) in self.files).lower()
        if command == "download":
            Path(args[1]).write_bytes(self.files[str(args[0])])
            return ""
        if command == "tar-out":
            with tarfile.open(args[1], "w") as archive:
                for name, data in self.modules.items():
                    info = tarfile.TarInfo(name)
                    info.size = len(data)
                    archive.addfile(info, io.BytesIO(data))
            return ""
        raise AssertionError((command, args))

    def run_command(self, args, *, env=None):
        name = Path(args[0]).name
        if name == "unmkinitramfs":
            shutil.copytree(self.initrd, Path(args[-1]) / "main", symlinks=True)
            return ""
        if name == "readelf" and args[1] != "--version":
            self.assertEqual({"LC_ALL": "C"}, env)
            return " (NEEDED) Shared library: [libfixture.so]\n[Requesting program interpreter: /lib/fixture-loader]\n" if Path(args[-1]).read_bytes().startswith(b"ELF tool") else ""
        return "fixture-version"

    def extract(self):
        self.receipt.write_text(json.dumps(self.build))
        with patch.object(self.api, "Guest", self.guest), patch.object(self.api, "run", self.run_command), \
             patch.object(self.api.shutil, "which", side_effect=lambda n: str(self.root / n)):
            return self.api.extract_boot_assets(self.value, self.image, self.receipt, self.root / "assets", source=self.root)

    def test_extracts_final_pair_and_five_guest_tools_with_dependency_proof(self):
        result = self.extract()
        self.assertEqual("bridge-guest-boot-assets/v1", result["schemaVersion"])
        self.assertEqual(self.build["sha256"], result["diskSha256"])
        self.assertEqual(sha(self.receipt.read_bytes()), result["buildReceiptSha256"])
        self.assertEqual({"kind": "filesystem-uuid", "value": self.uuid}, result["rootSelector"])
        self.assertIn("root=UUID=" + self.uuid, result["commandLine"])
        self.assertEqual(sha(self.files["/boot/config-" + self.release]), result["kernelConfigSha256"])
        self.assertEqual(set(TOOLS), {t["initrdPath"] for t in result["initrdTools"]})
        for record in result["initrdTools"]:
            self.assertEqual(record["guestSha256"], record["initrdSha256"])
            self.assertEqual("regular", record["initrdType"])
            self.assertTrue(record["dependencies"])
        self.assertEqual({"guestfish", "unmkinitramfs", "readelf", "dpkg-query"}, {t["name"] for t in result["extractionTools"]})
        self.assertFalse(result["publicationReady"])
        self.assertTrue((self.root / "assets/boot-assets.json").is_file())

    def test_default_grub_pair_is_selected_without_optional_root_kernel_link(self):
        del self.aliases["/vmlinuz"]
        result = self.extract()
        self.assertEqual(self.release, result["kernelRelease"])
        self.assertEqual(sha(self.files["/boot/vmlinuz-" + self.release]), result["kernel"]["sha256"])
        self.assertNotIn(("realpath", "/vmlinuz"), self.guest_calls)

    def test_grub_kernel_path_cannot_alias_a_different_release(self):
        self.aliases["/boot/vmlinuz-" + self.release] = "/boot/vmlinuz-another-amd64"
        with self.assertRaisesRegex(ValueError, "canonical"):
            self.extract()
        self.assertFalse((self.root / "assets/boot-assets.json").exists())

    def test_root_kernel_link_must_agree_with_default_grub_entry_when_present(self):
        self.aliases["/vmlinuz"] = "/boot/vmlinuz-another-amd64"
        with self.assertRaisesRegex(ValueError, "kernel"):
            self.extract()
        self.assertFalse((self.root / "assets/boot-assets.json").exists())

    def test_conditional_kernel_is_not_a_determinate_default(self):
        del self.aliases["/vmlinuz"]
        self.files["/boot/grub/grub.cfg"] = self.files["/boot/grub/grub.cfg"].replace(
            b"linux ", b"if [ 1 = 0 ]; then\nlinux ") + b"fi\n"
        with self.assertRaisesRegex(ValueError, "conditional"):
            self.extract()
        self.assertFalse((self.root / "assets/boot-assets.json").exists())

    def test_conditional_initrd_is_not_a_determinate_default(self):
        del self.aliases["/vmlinuz"]
        self.files["/boot/grub/grub.cfg"] = self.files["/boot/grub/grub.cfg"].replace(
            b"initrd ", b"if [ 1 = 0 ]; then\ninitrd ") + b"fi\n"
        with self.assertRaisesRegex(ValueError, "conditional"):
            self.extract()

    def test_second_kernel_in_default_entry_is_ambiguous(self):
        del self.aliases["/vmlinuz"]
        self.files["/boot/grub/grub.cfg"] += b"linux /boot/vmlinuz-another-amd64\n"
        with self.assertRaisesRegex(ValueError, "ambiguous"):
            self.extract()

    def test_non_boot_conditionals_can_precede_unconditional_default_pair(self):
        del self.aliases["/vmlinuz"]
        self.files["/boot/grub/grub.cfg"] = self.files["/boot/grub/grub.cfg"].replace(
            b"linux ", b"if [ x = y ]; then\ninsmod foo\nelse\ninsmod bar\nfi\nlinux ")
        self.assertEqual(self.release, self.extract()["kernelRelease"])

    def test_debian_inline_xen_module_condition_keeps_default_boot_pair(self):
        del self.aliases["/vmlinuz"]
        self.files["/boot/grub/grub.cfg"] = self.files["/boot/grub/grub.cfg"].replace(
            b"linux ", b"if [ x$grub_platform = xxen ]; then insmod xzio; insmod lzopio; fi\nlinux ")
        self.assertEqual(self.release, self.extract()["kernelRelease"])

    def test_loop_around_default_boot_commands_is_unsupported(self):
        del self.aliases["/vmlinuz"]
        self.files["/boot/grub/grub.cfg"] = self.files["/boot/grub/grub.cfg"].replace(
            b"linux ", b"while false; do\nlinux ") + b"done\n"
        with self.assertRaisesRegex(ValueError, "unsupported"):
            self.extract()

    def test_compound_line_cannot_hide_a_second_kernel_command(self):
        del self.aliases["/vmlinuz"]
        self.files["/boot/grub/grub.cfg"] += b"true; linux /boot/vmlinuz-another-amd64\n"
        with self.assertRaisesRegex(ValueError, "unsupported|ambiguous"):
            self.extract()

    def test_default_menu_cannot_itself_be_conditional(self):
        del self.aliases["/vmlinuz"]
        self.files["/boot/grub/grub.cfg"] = b"if [ 1 = 0 ]; then\n" + self.files["/boot/grub/grub.cfg"] + b"}\nfi\n"
        with self.assertRaisesRegex(ValueError, "conditional"):
            self.extract()

    def test_default_menu_cannot_be_inside_a_loop(self):
        del self.aliases["/vmlinuz"]
        self.files["/boot/grub/grub.cfg"] = b"while false; do\n" + self.files["/boot/grub/grub.cfg"] + b"}\ndone\n"
        with self.assertRaisesRegex(ValueError, "conditional"):
            self.extract()

    def test_default_menu_cannot_be_a_function_definition(self):
        del self.aliases["/vmlinuz"]
        self.files["/boot/grub/grub.cfg"] = b"function unused {\n" + self.files["/boot/grub/grub.cfg"] + b"}\n}\n"
        with self.assertRaisesRegex(ValueError, "conditional"):
            self.extract()

    def test_unbalanced_prefix_cannot_cancel_an_open_conditional(self):
        del self.aliases["/vmlinuz"]
        self.files["/boot/grub/grub.cfg"] = b"fi\nif [ 1 = 0 ]; then\n" + self.files["/boot/grub/grub.cfg"] + b"}\nfi\n"
        with self.assertRaisesRegex(ValueError, "conditional"):
            self.extract()

    def test_quoted_semicolon_cannot_fabricate_an_initrd_statement(self):
        del self.aliases["/vmlinuz"]
        self.files["/boot/grub/grub.cfg"] = self.files["/boot/grub/grub.cfg"].replace(b"\ninitrd ", b" ';' initrd ")
        with self.assertRaisesRegex(ValueError, "initrd|unsupported"):
            self.extract()

    def test_escaped_semicolon_cannot_fabricate_an_initrd_statement(self):
        del self.aliases["/vmlinuz"]
        self.files["/boot/grub/grub.cfg"] = self.files["/boot/grub/grub.cfg"].replace(b"\ninitrd ", rb" \; initrd ")
        with self.assertRaisesRegex(ValueError, "initrd|unsupported"):
            self.extract()

    def test_condition_cannot_execute_another_kernel_command(self):
        del self.aliases["/vmlinuz"]
        self.files["/boot/grub/grub.cfg"] += b"if linux /boot/vmlinuz-another-amd64; then true; fi\n"
        with self.assertRaisesRegex(ValueError, "unsupported"):
            self.extract()

    def test_inline_hash_cannot_hide_a_second_kernel_command(self):
        del self.aliases["/vmlinuz"]
        self.files["/boot/grub/grub.cfg"] += b"true x#; linux /boot/vmlinuz-another-amd64\n"
        with self.assertRaisesRegex(ValueError, "unsupported|ambiguous"):
            self.extract()

    def test_regular_root_kernel_file_cannot_conflict_with_grub(self):
        del self.aliases["/vmlinuz"]
        self.files["/vmlinuz"] = b"regular root kernel"
        with self.assertRaisesRegex(ValueError, "kernel"):
            self.extract()

    def test_corrupt_payload_or_mixed_build_refuses_before_output(self):
        self.build["sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "payload"):
            self.extract()
        self.assertFalse((self.root / "assets/boot-assets.json").exists())

    def test_final_tool_mismatch_leaves_no_completion_receipt(self):
        (self.initrd / "bin/mount").write_bytes(b"wrong mount")
        with self.assertRaisesRegex(ValueError, "initrd.*bytes"):
            self.extract()
        self.assertFalse((self.root / "assets/boot-assets.json").exists())

    def test_initrd_tool_leaf_symlink_is_not_a_canonical_copy(self):
        path = self.initrd / "sbin/blkid"
        path.unlink()
        path.symlink_to("blockdev")
        with self.assertRaisesRegex(ValueError, "regular"):
            self.extract()

    def test_missing_dependency_is_not_a_tool_match(self):
        (self.initrd / "lib/libfixture.so").unlink()
        with self.assertRaisesRegex(ValueError, "dependency"):
            self.extract()

    def test_wrong_root_or_kernel_selection_is_refused(self):
        self.uuid = "not-a-uuid"
        with self.assertRaisesRegex(ValueError, "UUID"):
            self.extract()

    def test_wrong_default_kernel_and_saved_override_are_refused(self):
        self.files["/boot/grub/grub.cfg"] = b'set default="0"\nmenuentry Linux {\nlinux /boot/vmlinuz-other root=UUID=x console=ttyS0,115200n8\n'
        with self.assertRaisesRegex(ValueError, "default kernel"):
            self.extract()

    def test_missing_initrd_endpoint_is_refused(self):
        (self.initrd / "usr/lib/bridge/bridge-guest").unlink()
        with self.assertRaises((ValueError, FileNotFoundError)):
            self.extract()

    def test_selected_kernel_module_must_be_present_when_not_builtin(self):
        self.files["/boot/config-" + self.release] = self.files["/boot/config-" + self.release].replace(b"CONFIG_VIRTIO_BLK=y", b"CONFIG_VIRTIO_BLK=m")
        with self.assertRaisesRegex(ValueError, "driver"):
            self.extract()

    def test_unsupported_kernel_format_is_not_silently_converted(self):
        self.files["/boot/vmlinuz-" + self.release] = b"gzip or EFI wrapper"
        with self.assertRaisesRegex(ValueError, "kernel format"):
            self.extract()

    def test_initrd_module_bytes_match_the_selected_guest_tree(self):
        config = "/boot/config-" + self.release
        self.files[config] = self.files[config].replace(b"CONFIG_VIRTIO_BLK=y", b"CONFIG_VIRTIO_BLK=m")
        path = self.initrd / "lib/modules" / self.release / "kernel/virtio_blk.ko"
        path.parent.mkdir(parents=True)
        path.write_bytes(b"wrong release module")
        self.modules["kernel/virtio_blk.ko"] = b"selected module"
        with self.assertRaisesRegex(ValueError, "module.*bytes"):
            self.extract()

    def test_saved_grub_override_cannot_select_an_old_kernel(self):
        self.files["/boot/grub/grubenv"] += b"next_entry=old kernel\n"
        with self.assertRaisesRegex(ValueError, "override"):
            self.extract()

    def test_dependency_bytes_must_match_guest_not_just_exist(self):
        (self.initrd / "lib/libfixture.so").write_bytes(b"ELF different")
        with self.assertRaisesRegex(ValueError, "bytes"):
            self.extract()

    def test_rooted_absolute_links_never_resolve_against_the_host(self):
        inspect = importlib.import_module("guest.boot_inspect")
        path = self.initrd / "usr/bin/findmnt"
        path.unlink()
        path.symlink_to("/sbin/blkid")
        self.assertEqual(self.initrd / "usr/sbin/blkid", inspect.rooted_file(self.initrd, "/usr/bin/findmnt"))
        path.unlink()
        path.symlink_to("../../../../outside")
        with self.assertRaisesRegex(ValueError, "escapes"):
            inspect.rooted_file(self.initrd, "/usr/bin/findmnt")

    def test_existing_or_partial_output_cannot_be_reused(self):
        (self.root / "assets").mkdir()
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.extract()

    def test_arm_pair_uses_pinned_kernel_and_hvc0(self):
        policy = self.api.build.VERSIONS["arm64Kernel"]
        old, self.release = self.release, policy["release"]
        self.files = {k.replace(old, self.release): v for k, v in self.files.items()}
        header = bytearray(4096)
        header[0x38:0x3c] = b"ARM\x64"
        self.files["/boot/vmlinuz-" + self.release] = bytes(header)
        self.aliases["/vmlinuz"] = "/boot/vmlinuz-" + self.release
        self.files["/boot/grub/grub.cfg"] = ('set default="0"\nmenuentry Linux {\nlinux /boot/vmlinuz-' + self.release + " root=PARTUUID=old ro console=hvc0 noresume\ninitrd /boot/initrd.img-" + self.release + "\n").encode()
        self.value["architecture"] = self.build["architecture"] = "arm64"
        self.value["nodeLocalRef"] = self.value["nodeLocalRef"].rsplit("-", 1)[0] + "-arm64"
        self.value["packages"].append({"name": policy["package"], "version": policy["version"], "path": "kernel.deb", "sha256": policy["packageSha256"]})
        self.image = self.image.rename(self.root / "os.raw")
        self.build["payload"] = self.image.name
        self.build["inputSha256"] = sha(self.api.encoded(self.value))
        # Only the synthetic kernel's expected byte hash is substituted. No production pin changes.
        with patch.dict(policy, {"kernelSha256": sha(header)}):
            result = self.extract()
        self.assertEqual("linux-arm64-image", result["kernel"]["format"])
        self.assertEqual(self.release, result["kernelRelease"])
        self.assertIn("console=hvc0", result["commandLine"])

    def test_module_driver_can_be_verified_against_guest_bytes(self):
        config = "/boot/config-" + self.release
        self.files[config] = self.files[config].replace(b"CONFIG_VIRTIO_BLK=y", b"CONFIG_VIRTIO_BLK=m")
        path = self.initrd / "lib/modules" / self.release / "kernel/virtio_blk.ko.xz"
        path.parent.mkdir(parents=True)
        path.write_bytes(b"selected module")
        self.modules["kernel/virtio_blk.ko.xz"] = path.read_bytes()
        result = self.extract()
        self.assertIn({"name": "virtio_blk", "builtIn": False}, result["requiredDrivers"])

    def test_early_hook_and_endpoint_each_match_producer_and_guest(self):
        self.build["runtime"]["binarySha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "endpoint"):
            self.extract()

    def test_changed_input_identity_does_not_inherit_build_provenance(self):
        self.build["inputSha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "identity"):
            self.extract()

    def test_cli_failure_has_nonzero_status_and_no_success_stdout(self):
        import contextlib
        stdout, stderr = io.StringIO(), io.StringIO()
        args = ["boot_assets", "--lock", str(self.root / "missing.json"), "--image", str(self.image),
                "--build-receipt", str(self.receipt), "--out", str(self.root / "assets")]
        with patch("sys.argv", args), contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            self.assertEqual(2, self.api.main())
        self.assertEqual("", stdout.getvalue())
        self.assertIn("guest boot assets:", stderr.getvalue())

    def test_modular_virtio_rng_uses_linux_hyphen_alias(self):
        config = "/boot/config-" + self.release
        self.files[config] = self.files[config].replace(b"CONFIG_HW_RANDOM_VIRTIO=y", b"CONFIG_HW_RANDOM_VIRTIO=m")
        path = self.initrd / "lib/modules" / self.release / "kernel/virtio-rng.ko.xz"
        path.parent.mkdir(parents=True)
        path.write_bytes(b"selected rng module")
        self.modules["kernel/virtio-rng.ko.xz"] = path.read_bytes()
        self.assertIn({"name": "virtio_rng", "builtIn": False}, self.extract()["requiredDrivers"])

    def test_default_initrd_must_match_exported_release(self):
        path = "/boot/grub/grub.cfg"
        self.files[path] = self.files[path].replace(("initrd /boot/initrd.img-" + self.release).encode(), b"initrd /boot/initrd.img-old")
        with self.assertRaisesRegex(ValueError, "default initrd"):
            self.extract()

    def test_nonobject_build_receipt_has_a_validation_error(self):
        with patch.object(self, "build", []), self.assertRaisesRegex(ValueError, "receipt"):
            self.extract()

    def test_cli_malformed_module_archive_exits_two(self):
        import contextlib
        args = ["boot_assets", "--lock", "fixture", "--image", str(self.image), "--build-receipt", str(self.receipt), "--out", str(self.root / "assets")]
        with patch("sys.argv", args), patch.object(self.api, "load_lock", return_value=self.value), \
             patch.object(self.api, "extract_boot_assets", side_effect=tarfile.ReadError("bad archive")), \
             contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(2, self.api.main())

    def test_nonzero_grub_default_does_not_inherit_first_linux(self):
        path = "/boot/grub/grub.cfg"
        self.files[path] = self.files[path].replace(b'set default="0"', b'set default="1"')
        with self.assertRaisesRegex(ValueError, "default"):
            self.extract()

    def test_nonlinux_first_menuentry_cannot_be_skipped(self):
        path = "/boot/grub/grub.cfg"
        self.files[path] = b'set default="0"\nmenuentry Other {\nchainloader +1\n}\n' + self.files[path]
        with self.assertRaisesRegex(ValueError, "default"):
            self.extract()

    def test_tool_requires_executable_permissions(self):
        (self.initrd / "sbin/blkid").chmod(0o644)
        with self.assertRaisesRegex(ValueError, "executable"):
            self.extract()

    def test_endpoint_requires_executable_permissions(self):
        (self.initrd / "usr/lib/bridge/bridge-guest").chmod(0o644)
        with self.assertRaisesRegex(ValueError, "executable"):
            self.extract()

    def test_early_hook_requires_executable_permissions(self):
        (self.initrd / "scripts/local-bottom/bridge-state").chmod(0o644)
        with self.assertRaisesRegex(ValueError, "executable"):
            self.extract()

    def test_inspection_locale_does_not_change_version_pins(self):
        import sys
        with patch.dict(os.environ, {"LC_ALL": "fr_FR.UTF-8"}):
            command = [sys.executable, "-c", "import os; print(os.environ['LC_ALL'])"]
            result = self.api.run(command, env={"LC_ALL": "C"})
            self.assertEqual("fr_FR.UTF-8", self.api.run(command))
        self.assertEqual("C", result)

    def test_generated_conditional_default_zero_is_supported(self):
        path = "/boot/grub/grub.cfg"
        self.files[path] = self.files[path].replace(b'set default="0"', b'if [ "${next_entry}" ]; then\nset default="${next_entry}"\nelse\nset default="0"\nfi')
        self.assertTrue(self.extract()["kernel"])

    def test_elf_interpreter_requires_execute_but_shared_library_does_not(self):
        (self.initrd / "lib/fixture-loader").chmod(0o644)
        with self.assertRaisesRegex(ValueError, "executable"):
            self.extract()

    def test_guestfish_interface_is_read_only(self):
        with patch.object(self.api, "run", return_value="root") as run:
            guest = self.api.Guest(self.image, Path("/pinned/guestfish"))
            guest.call("inspect-get-roots")
        self.assertEqual(["/pinned/guestfish", "--ro", "-a", str(self.image), "-i", "inspect-get-roots"], list(map(str, run.call_args.args[0])))
