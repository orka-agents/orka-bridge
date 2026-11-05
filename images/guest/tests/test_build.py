"""Offline recipe checks; actual disk/daemon evidence comes from the base boot."""
import importlib
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from .fixtures import lock

HOOK = Path(__file__).resolve().parents[1] / "rootfs/initramfs/hooks/bridge-state"
HOOK_FUNCTIONS = ". /usr/share/initramfs-tools/hook-functions"
# initramfs-tools' copy_exec with sources under $HOST: a /bin or /sbin target
# is usr-merged, and an existing target is kept but still reported as success.
COPY_EXEC = r'''
copy_exec() {
    src=$1
    case $src in "$HOST"/*) ;; *) src=$HOST$src ;; esac
    target=${2:-$1}
    case $target in /bin/*|/sbin/*) target=/usr$target ;; esac
    [ -f "$src" ] || return 1
    [ -e "$DESTDIR$target" ] && return 0
    mkdir -p "$DESTDIR${target%/*}"
    cp "$src" "$DESTDIR$target"
}
manual_add_modules() { :; }
readlink() { command readlink "$1" "$HOST$2"; }
'''
# Endpoint path -> canonical util-linux/e2fsprogs file on a usr-merged guest.
ENDPOINT_TOOLS = {"/sbin/blkid": "usr/sbin/blkid", "/sbin/blockdev": "usr/sbin/blockdev", "/sbin/mkfs.ext4": "usr/sbin/mke2fs",
                  "/bin/mount": "usr/bin/mount", "/usr/bin/findmnt": "usr/bin/findmnt"}


class InitramfsHookTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name).resolve()
        self.host, self.dest = root / "host", root / "initramfs"
        for tree in (self.host, self.dest):
            for name in ("bin", "sbin"):
                (tree / "usr" / name).mkdir(parents=True)
                (tree / name).symlink_to("usr/" + name)
        (self.host / "usr/lib/bridge").mkdir(parents=True)
        for name in [*ENDPOINT_TOOLS.values(), "usr/lib/bridge/bridge-guest"]:
            (self.host / name).write_bytes(b"guest " + name.encode())
        (self.host / "usr/sbin/mkfs.ext4").symlink_to("mke2fs")

    def run_hook(self, functions=COPY_EXEC, destdir=True):
        hook = HOOK.read_text()
        self.assertIn(HOOK_FUNCTIONS, hook)
        env = {"PATH": os.environ["PATH"], "HOST": str(self.host)}
        if destdir:
            env["DESTDIR"] = str(self.dest)
        return subprocess.run(["/bin/sh", "-c", hook.replace(HOOK_FUNCTIONS, functions), "hook"], env=env,
                              capture_output=True, text=True, timeout=10, check=False)

    def assert_endpoint_tools(self):
        for path, source in ENDPOINT_TOOLS.items():
            installed = self.dest / path.lstrip("/")
            self.assertFalse(installed.is_symlink(), path)
            self.assertEqual((self.host / source).read_bytes(), installed.read_bytes(), path)

    def test_klibc_mount_is_replaced_without_busybox(self):
        # klibc-utils' hook runs first and its mount has no --bind.
        (self.dest / "usr/bin/mount").write_bytes(b"klibc mount")
        result = self.run_hook()
        self.assertEqual(0, result.returncode, result.stderr)
        self.assert_endpoint_tools()

    def test_busybox_applets_are_replaced_without_changing_busybox(self):
        busybox = self.dest / "usr/bin/busybox"
        busybox.write_bytes(b"busybox")
        for applet in ("usr/bin/mount", "usr/sbin/blockdev", "usr/sbin/blkid", "usr/sbin/mke2fs"):
            os.link(busybox, self.dest / applet)
        result = self.run_hook()
        self.assertEqual(0, result.returncode, result.stderr)
        self.assert_endpoint_tools()
        self.assertEqual(b"busybox", busybox.read_bytes())
        self.assertEqual(b"busybox", (self.dest / "usr/sbin/mke2fs").read_bytes())

    def test_endpoint_tool_that_is_not_installed_fails_build(self):
        skipped = COPY_EXEC.replace('[ -e "$DESTDIR$target" ] && return 0', 'case $target in /usr/bin/mount) return 0 ;; esac')
        result = self.run_hook(skipped)
        self.assertNotEqual(0, result.returncode)
        self.assertIn("E: bridge-state: /bin/mount", result.stderr)

    def test_missing_destdir_never_touches_builder_paths(self):
        trace = self.host / "trace"
        recorded = 'copy_exec() { echo "copy $*" >> "$HOST/trace"; }\nrm() { echo "rm $*" >> "$HOST/trace"; }\nreadlink() { echo "$2"; }\nmanual_add_modules() { :; }\n'
        result = self.run_hook(recorded, destdir=False)
        self.assertNotEqual(0, result.returncode)
        self.assertNotIn("rm ", trace.read_text() if trace.exists() else "")


class BuildTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.value = lock(self.root)
        from .test_runtime import runtime_inputs
        runtime_inputs(self.root, self.value)
        self.api = importlib.import_module("guest.build")

    def test_runtime_inputs_are_verified_without_claiming_application_cache(self):
        result = self.api.verify_os_inputs(self.value, source=self.root)
        self.assertFalse(result["componentCacheVerified"])
        self.assertFalse(result["publicationReady"])
        self.assertEqual(self.root / "base.qcow2", result["base"])

    def test_h1_capacities_and_formats(self):
        for arch, fmt in (("amd64", "qcow2"), ("arm64", "raw")):
            contract = self.api.disk_contract(arch)
            self.assertEqual(fmt, contract["osDisk"]["format"])
            self.assertEqual(16 * 1024**3, contract["osDisk"]["virtualBytes"])
            self.assertEqual(32 * 1024**3, contract["stateDisk"]["virtualBytes"])
            self.assertEqual("create-empty", contract["stateDisk"]["creation"])

    def test_rootfs_staging_starts_no_services(self):
        with patch("subprocess.Popen", side_effect=AssertionError("no process during staging")):
            result = self.api.stage_rootfs(self.value, source=self.root, out=self.root / "overlay")
        self.assertFalse(result["diskBuilt"])
        self.assertFalse(result["bootVerified"])
        root = self.root / "overlay"
        self.assertTrue((root / "usr/lib/bridge/bridge-guest").is_file())
        self.assertFalse((root / "usr/lib/bridge/bridge-early-state").exists())
        self.assertTrue((root / "etc/systemd/system/multi-user.target.wants/bridge-guest.service").is_symlink())
        self.assertFalse((root / "run/bridge/boot.json").exists())
        self.assertTrue((root / "var/lib/bridge").is_dir())
        self.assertEqual([], list((root / "var/lib/bridge").iterdir()))

    def test_native_payload_installs_only_runtime_modules_and_native_archives(self):
        from .test_offline import native_inputs
        from .test_runtime import runtime_inputs
        value, _ = native_inputs(self.root)
        runtime_inputs(self.root, value)
        self.api.stage_rootfs(value, source=self.root, out=self.root / 'native')
        root = self.root / 'native'
        self.assertTrue((root / 'opt/bridge/guest/lifecycle.py').is_file())
        self.assertTrue((root / 'opt/bridge/bundle/install.json').is_file())
        self.assertTrue((root / 'usr/local/bin/kmx').is_symlink())
        self.assertTrue((root / 'etc/systemd/system/multi-user.target.wants/bridge-infrastructure.service').is_symlink())
        for name in ('boot_test.py', 'test_seed.py', 'reference_host.py'):
            self.assertFalse((root / 'opt/bridge/guest' / name).exists())
        self.assertFalse((root / 'opt/bridge/bundle/components/telegram.oci.tar').exists())
        self.assertEqual([], list((root / 'var/lib/bridge').iterdir()))

    def test_archive_has_guest_root_ownership_not_builder_ownership(self):
        import tarfile
        self.assertTrue(callable(getattr(self.api, "rootfs_archive", None)), "normalized rootfs archive missing")
        overlay = self.root / "overlay"
        self.api.stage_rootfs(self.value, source=self.root, out=overlay)
        (overlay / "etc").chmod(0o700)
        archive = self.root / "rootfs.tar"
        self.api.rootfs_archive(overlay, archive)
        with tarfile.open(archive) as tar:
            self.assertTrue(all(m.uid == m.gid == 0 for m in tar))
            self.assertEqual(0o755, tar.getmember("etc").mode)
            self.assertEqual(0o755, tar.getmember("usr/lib/bridge/check-state").mode)

    def test_initramfs_dependency_copy_failure_fails_build(self):
        import subprocess
        hook = (Path(self.api.__file__).with_name("rootfs") / "initramfs/hooks/bridge-state").read_text()
        hook = hook.replace(". /usr/share/initramfs-tools/hook-functions", "copy_exec() { return 1; }; manual_add_modules() { return 0; }")
        result = subprocess.run(["/bin/sh", "-c", hook, "hook", "build"], capture_output=True, check=False)
        self.assertNotEqual(0, result.returncode)

    def test_package_mutation_after_preflight_cannot_change_selected_bytes(self):
        import hashlib
        package = dict(self.value["packages"][0], name="docker.io", version="1.0", path="docker.deb", sha256=hashlib.sha256(b"selected").hexdigest())
        (self.root / package["path"]).write_bytes(b"selected")
        self.value["packages"].append(package)
        self.api.verify_os_inputs(self.value, source=self.root)
        source = self.root / package["path"]
        source.write_bytes(b"replacement after verification")
        self.assertTrue(callable(getattr(self.api, "stage_package", None)), "locked package snapshot missing")
        with self.assertRaisesRegex(ValueError, "package changed"):
            self.api.stage_package(source, self.root / "staged.deb", package["sha256"])

    def test_fixed_debian_layout_rejects_other_roots_or_extra_filesystems(self):
        self.assertTrue(callable(getattr(self.api, "root_esp", None)), "explicit base layout validation missing")
        self.assertEqual("/dev/sda2", self.api.root_esp(["/dev/sda1: unknown", "/dev/sda2: vfat", "/dev/sda3: ext4"], "/dev/sda3"))
        for filesystems in (["/dev/sda2: vfat", "/dev/sda3: xfs"], ["/dev/sda2: vfat", "/dev/sda3: ext4", "/dev/sda4: ext4"]):
            with self.assertRaises(ValueError):
                self.api.root_esp(filesystems, "/dev/sda3")

    def test_base_reads_use_an_owned_verified_snapshot(self):
        import hashlib
        from unittest.mock import patch
        tool = self.root / "builder.bin"
        tool.write_bytes(b"builder")
        for name in ("qemu-img", "guestfish", "virt-resize", "virt-customize"):
            self.value["tools"].append({"name": name, "version": "1.0", "path": tool.name, "sha256": hashlib.sha256(tool.read_bytes()).hexdigest()})
        package = next(p for p in self.value["packages"] if p["name"] == "debian-base")
        original = self.root / package["path"]
        def inspect(snapshot):
            self.assertNotEqual(original, snapshot)
            self.assertEqual(package["sha256"], self.api.file_hash(snapshot))
            original.write_bytes(b"external replacement after snapshot")
            self.assertEqual(package["sha256"], self.api.file_hash(snapshot))
            raise RuntimeError("snapshot observed")
        with patch.object(self.api.platform, "machine", return_value="x86_64"), patch.object(self.api.shutil, "which", return_value=str(tool)), patch.object(self.api, "run", return_value="1.0"), patch.object(self.api, "image_info", side_effect=inspect), self.assertRaisesRegex(RuntimeError, "snapshot observed"):
            self.api.build(self.value, source=self.root, out=self.root / "build")

    def test_bad_input_leaves_no_output(self):
        (self.root / "base.qcow2").write_bytes(b"corrupt")
        with self.assertRaisesRegex(ValueError, "checksum"):
            self.api.stage_rootfs(self.value, source=self.root, out=self.root / "overlay")
        self.assertFalse((self.root / "overlay").exists())

    def test_existing_output_is_not_overwritten(self):
        with self.assertRaisesRegex(ValueError, "already exist"):
            self.api.stage_rootfs(self.value, source=self.root, out=self.root)

    def test_factory_overlay_has_no_test_bootstrap(self):
        self.api.stage_rootfs(self.value, source=self.root, out=self.root / "overlay")
        names = [str(p.relative_to(self.root / "overlay")) for p in (self.root / "overlay").rglob("*")]
        self.assertFalse(any("reference-seed" in n or "reference-check" in n for n in names))

    def test_cloud_init_masks_cover_current_and_legacy_units(self):
        expected = {"cloud-init-main.service", "cloud-init-network.service", "cloud-init.service",
                    "cloud-init-local.service", "cloud-config.service", "cloud-final.service"}
        prefix = "run-command systemctl mask "
        for architecture in ("amd64", "arm64"):
            with self.subTest(architecture=architecture):
                commands = self.api.customize_commands([], architecture)
                masks = [command.removeprefix(prefix).split() for command in commands if command.startswith(prefix)]
                self.assertEqual(1, len(masks))
                self.assertEqual(expected, {unit for unit in masks[0] if unit.startswith("cloud-")})

    def test_assembly_command_is_offline_and_never_uses_apt(self):
        self.assertTrue(callable(getattr(self.api, "customize_commands", None)))
        commands = self.api.customize_commands([self.root / "docker.deb"], "amd64")
        joined = "\n".join(commands)
        self.assertIn("dpkg --force-confold -i", joined)
        self.assertNotIn("apt-get", joined)
        self.assertIn("policy-rc.d", joined)
        self.assertIn("update-initramfs", joined)
        self.assertIn("--removable --no-nvram", joined)
        self.assertNotIn("kind create", joined)
