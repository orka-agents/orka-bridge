"""Export and inspect boot assets from a final unbooted payload; never boot it."""
import argparse
import hashlib
import os
import re
import shlex
import shutil
import subprocess
import sys
import tarfile
import tempfile
import uuid
from pathlib import Path

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    __package__ = "guest"
from . import build
from .boot_inspect import (
    DRIVERS,
    ENDPOINT_TOOLS,
    compare_guest_file,
    dependencies,
    module_hashes,
    module_identity,
    rooted_file,
)
from .lock import (
    checked_file,
    file_hash,
    load_lock,
    matches,
    read_json,
    require,
    validate_lock,
)

encoded = build.encoded


def run(argv, *, env=None):
    environment = None if env is None else {**os.environ, **env}
    return subprocess.run([str(x) for x in argv], env=environment, check=True, capture_output=True, text=True).stdout.strip()


class Guest:
    def __init__(self, image, executable):
        self.image, self.executable = image, executable

    def call(self, command, *args):
        return run([self.executable, "--ro", "-a", self.image, "-i", command, *args])


def extraction_tools(lock, source):
    selected = {t["name"]: t for t in lock["tools"]}
    paths, records = {}, []
    for name in ("guestfish", "unmkinitramfs", "readelf", "dpkg-query"):
        require(name in selected, "missing pinned extraction tool: " + name)
        checked_file(source, selected[name]["path"], selected[name]["sha256"])
        path = shutil.which(name)
        if path is None:
            raise ValueError("missing installed extraction tool: " + name)
        require(file_hash(Path(path)) == selected[name]["sha256"], "extraction tool checksum mismatch: " + name)
        paths[name] = Path(path)
    for name, path in paths.items():
        # unmkinitramfs has no --version switch; bind its owning package version.
        version = run([paths["dpkg-query"], "-W", "-f=${Version}", "initramfs-tools-core"]) if name == "unmkinitramfs" else run([path, "--version"])
        require(version == selected[name]["version"], "extraction tool version mismatch: " + name)
        records.append({"name": name, "version": version, "executableSha256": selected[name]["sha256"]})
    return paths, records


def kernel_format(file, architecture):
    with file.open("rb") as stream:
        header = stream.read(4096)
    if architecture == "amd64":
        require(header[0x1fe:0x200] == b"\x55\xaa" and header[0x202:0x206] == b"HdrS", "unsupported AMD64 kernel format")
        return "linux-bzimage"
    require(header[0x38:0x3c] == b"ARM\x64", "unsupported ARM64 kernel format; owner acceptance required before derivation")
    return "linux-arm64-image"


def initrd_compression(file):
    with file.open("rb") as stream:
        header = stream.read(6)
    for prefix, name in ((b"\x1f\x8b", "gzip"), (b"\x28\xb5\x2f\xfd", "zstd"), (b"\xfd7zXZ\x00", "xz"),
                         (b"070701", "cpio-prefix"), (b"070702", "cpio-prefix")):
        if header.startswith(prefix):
            return name  # cpio-prefix may contain a later compressed main archive; do not call it uncompressed.
    raise ValueError("unsupported initrd format")


def asset(file, **fields):
    require(file.is_file() and not file.is_symlink() and file.stat().st_size > 0, "missing or nonregular boot asset")
    return {"name": file.name, "bytes": file.stat().st_size, "sha256": file_hash(file), **fields}


def selected_kernel(guest, architecture, expected_grub):
    require(re.findall(r"(?m)^GRUB_DEFAULT=(.*)$", guest.call("cat", "/etc/default/grub")) == ["0"]
            and guest.call("checksum", "sha256", "/etc/default/grub") == expected_grub, "unsupported GRUB default policy")
    commands = []
    for raw in guest.call("cat", "/boot/grub/grub.cfg").splitlines():
        if raw.lstrip().startswith("#"):
            continue
        require("#" not in raw and "\\" not in raw, "unsupported GRUB inline comment, escape or line continuation")
        shlex.split(raw)  # Validate embedded quotes without discarding their context below.
        if ";" in raw and any(quote in raw for quote in ("'", '"')):
            require(raw.count(";") == 1 and re.fullmatch(r"\s*(?:if|elif)\s+\[[^;]*\]\s*;\s*then\s*", raw), "unsupported quoted compound GRUB command")
        lexer = shlex.shlex(raw, posix=False, punctuation_chars=";")
        lexer.commenters = ""
        lexer.whitespace_split = True
        statement = []
        for token in lexer:
            if set(token) == {";"}:
                if statement:
                    commands.append(statement)
                    statement = []
            else:
                statement.append(token)
        if statement:
            commands.append(statement)
    expanded = []
    for command in commands:
        if command[0] in ("then", "else", "do") and len(command) > 1:
            expanded.extend((command[:1], command[1:]))
        else:
            expanded.append(command)
    commands = expanded
    defaults = [shlex.split(" ".join(line))[1].removeprefix("default=") for line in commands
                if len(line) >= 2 and line[0] == "set" and line[1].startswith("default=")]
    require("0" in defaults and set(defaults) <= {"0", "${next_entry}"}, "unsupported GRUB default selection")
    index = next((i for i, line in enumerate(commands) if line[0] in ("linux", "linuxefi")), None)
    if index is None:
        raise ValueError("GRUB default kernel missing")
    menus = [i for i, command in enumerate(commands) if command[0] in ("menuentry", "submenu")]
    require(menus and commands[menus[0]][0] == "menuentry" and menus[0] < index
            and not any(command[0] in ("}", "menuentry", "submenu") for command in commands[menus[0] + 1:index]), "default menuentry is not the selected Linux kernel")
    prefix_conditionals = prefix_loops = prefix_blocks = 0
    for preceding in commands[:menus[0]]:
        if preceding[0] == "if":
            prefix_conditionals += 1
        elif preceding[0] == "fi":
            prefix_conditionals -= 1
        elif preceding[0] == "do":
            prefix_loops += 1
        elif preceding[0] == "done":
            prefix_loops -= 1
        prefix_blocks += preceding.count("{") - preceding.count("}")
        require(min(prefix_conditionals, prefix_loops, prefix_blocks) >= 0, "invalid GRUB prefix conditional")
    require(prefix_conditionals == prefix_loops == prefix_blocks == 0, "conditional GRUB default menu is unsupported")
    line = commands[index]
    require(len(line) >= 2 and matches(line[1], r"/boot/vmlinuz-[A-Za-z0-9._+-]{1,96}"), "invalid selected kernel path")
    kernel = guest.call("realpath", line[1])
    require(kernel == line[1], "GRUB default kernel path is not canonical")
    release = kernel.removeprefix("/boot/vmlinuz-")
    # Cloud images need not have Debian's optional root-level convenience link.
    if guest.call("is-symlink", "/vmlinuz") == "true" or guest.call("is-file", "/vmlinuz") == "true":
        require(guest.call("realpath", "/vmlinuz") == kernel, "GRUB default kernel differs from selected kernel")
    kernels, initrds, conditionals = [], [], 0
    for following in commands[menus[0] + 1:]:
        key = following[0]
        if key in ("menuentry", "submenu", "}"):
            break
        require(not any(marker in " ".join(following) for marker in ("&&", "||", "$(", "`")), "unsupported compound GRUB command")
        if key in ("then", "else"):
            require(conditionals > 0, "invalid GRUB conditional")
            following = following[1:]
            if not following:
                continue
            key = following[0]
        require(key in ("if", "elif", "fi", "load_video", "insmod", "set", "search", "echo", "recordfail", "savedefault", "true", "false", "test", "linux", "linuxefi", "initrd", "initrdefi"), "unsupported GRUB control flow or command")
        if key in ("if", "elif"):
            require(len(following) > 1 and following[1] in ("[", "test"), "unsupported executable GRUB condition")
        if key == "if":
            conditionals += 1
        elif key == "fi":
            require(conditionals > 0, "invalid GRUB conditional")
            conditionals -= 1
        elif key in ("else", "elif"):
            require(conditionals > 0, "invalid GRUB conditional")
        elif key in ("linux", "linuxefi", "initrd", "initrdefi"):
            require(conditionals == 0, "conditional GRUB boot command is unsupported")
            if key in ("linux", "linuxefi"):
                kernels.append(following)
            else:
                require(kernels == [line], "GRUB initrd command precedes its kernel")
                initrds.append(following[1:])
    require(conditionals == 0, "unterminated GRUB conditional")
    require(kernels == [line], "ambiguous GRUB default kernel commands")
    require(initrds == [["/boot/initrd.img-" + release]], "GRUB default initrd differs from exported release")
    console = "hvc0" if architecture == "arm64" else "ttyS0,115200n8"
    require([v for v in line if v.startswith("console=")] == ["console=" + console], "wrong kernel console")
    if guest.call("is-file", "/boot/grub/grubenv") == "true":
        require(not re.search(r"(?m)^(?:saved_entry|next_entry)=.+", guest.call("cat", "/boot/grub/grubenv")), "saved GRUB override is not allowed")
    return kernel, release, console


def inspect_pair(guest, lock, receipt, tools, out, work):
    architecture = lock["architecture"]
    roots = guest.call("inspect-get-roots").splitlines()
    require(len(roots) == 1 and roots[0].startswith("/dev/"), "expected one root filesystem")
    root_uuid = guest.call("vfs-uuid", roots[0])
    try:
        require(str(uuid.UUID(root_uuid)) == root_uuid, "noncanonical root UUID")
    except (ValueError, AttributeError) as error:
        raise ValueError("invalid root UUID") from error
    kernel_path, release, console = selected_kernel(guest, architecture, receipt["recipeFiles"]["/etc/default/grub"])
    kernel, initrd = out / "os.kernel", out / "os.initrd"
    guest.call("download", kernel_path, kernel)
    guest.call("download", "/boot/initrd.img-" + release, initrd)
    config_file = work / "kernel.config"
    guest.call("download", "/boot/config-" + release, config_file)
    config = config_file.read_text()
    policy = build.kernel_policy(lock)
    if policy:
        require(release == policy["release"] and file_hash(kernel) == policy["kernelSha256"], "installed ARM kernel differs from pinned package")
    fmt, compression = kernel_format(kernel, architecture), initrd_compression(initrd)
    modules = work / "modules.tar"
    guest.call("tar-out", "/lib/modules/" + release, modules)
    module_rows = module_identity(modules)
    expanded = work / "expanded"
    run([tools["unmkinitramfs"], initrd, expanded])
    root = expanded / "main" if (expanded / "main").is_dir() else expanded
    installed_modules = []
    try:
        tree = rooted_file(root, "/lib/modules/" + release, directory=True)
    except FileNotFoundError:
        tree = None
    if tree is not None:
        paths = [path for path in tree.rglob("*") if re.search(r"\.ko(?:\.(?:xz|gz|zst))?$", path.name)]
        hashes = module_hashes(modules, module_rows, {path.relative_to(tree).as_posix() for path in paths})
        for path in paths:
            require(path.is_file() and not path.is_symlink()
                    and hashes.get(path.relative_to(tree).as_posix()) == file_hash(path), "initrd module bytes differ from selected guest tree")
            installed_modules.append(path.name)
    drivers = []
    for name, symbol in DRIVERS.items():
        values = re.findall(r"(?m)^CONFIG_" + symbol + r"=([ym])$", config)
        require(len(values) == 1, "missing selected kernel driver: " + name)
        if values[0] == "m":
            require(any(re.fullmatch(re.escape(name) + r"\.ko(?:\.(?:xz|gz|zst))?", p.replace("-", "_")) for p in installed_modules), "missing initrd driver: " + name)
        drivers.append({"name": name, "builtIn": values[0] == "y"})
    scratch = work / "guest-file"
    endpoint = compare_guest_file(guest, root, "/usr/lib/bridge/bridge-guest", scratch, regular_leaf=True, executable=True)
    require(endpoint["guestSha256"] == receipt["runtime"]["binarySha256"], "endpoint differs from build")
    hook = rooted_file(root, "/scripts/local-bottom/bridge-state", regular_leaf=True, executable=True)
    early_path = "/etc/initramfs-tools/scripts/local-bottom/bridge-state"
    guest.call("download", early_path, scratch)
    require(file_hash(hook) == file_hash(scratch) == receipt["recipeFiles"][early_path], "early hook differs from guest/build")
    early_sha = file_hash(hook)
    hook_path = "/etc/initramfs-tools/hooks/bridge-state"
    guest.call("download", hook_path, scratch)
    require(file_hash(scratch) == receipt["recipeFiles"][hook_path], "initrd build hook differs from build")
    build_hook_sha = file_hash(scratch)
    checked = []
    for path in ENDPOINT_TOOLS:
        record = compare_guest_file(guest, root, path, scratch, regular_leaf=True, executable=True)
        executable = rooted_file(root, path, regular_leaf=True)
        record["dependencies"] = [compare_guest_file(guest, root, p, scratch) for p in dependencies(root, executable, architecture, tools["readelf"], run)]
        checked.append(record)
    return {"kernel": asset(kernel, format=fmt), "initrd": asset(initrd, format="initramfs", compression=compression),
            "rootSelector": {"kind": "filesystem-uuid", "value": root_uuid}, "kernelRelease": release,
            "commandLine": "root=UUID=" + root_uuid + " ro console=" + console + " noresume", "requiredDrivers": drivers,
            "modulesSha256": hashlib.sha256(encoded(sorted(module_rows))).hexdigest(),
            "kernelConfigSha256": file_hash(config_file), "endpointSha256": endpoint["guestSha256"],
            "earlyHookSha256": early_sha, "buildHookSha256": build_hook_sha, "initrdTools": checked}


def extract_boot_assets(lock, image, build_receipt, out, *, source):
    validate_lock(lock, production=False)
    require(not out.exists() and not out.is_symlink(), "boot output already exists")
    require(not image.is_symlink() and image.is_file(), "regular final payload required")
    require(not build_receipt.is_symlink() and build_receipt.is_file() and build_receipt.stat().st_size <= 1024**2, "invalid build receipt")
    data = build_receipt.read_bytes()
    receipt = read_json(data)
    require(isinstance(receipt, dict), "build receipt must be an object")
    require(receipt.get("schemaVersion") == "bridge-guest-build/v1" and receipt.get("architecture") == lock["architecture"]
            and receipt.get("payload") == image.name and image.name == ("os.raw" if lock["architecture"] == "arm64" else "os.qcow2")
            and receipt.get("inputSha256") == hashlib.sha256(encoded(lock)).hexdigest()
            and receipt.get("bootVerified") is False and receipt.get("publicationReady") is False, "build/input identity mismatch")
    tools, versions = extraction_tools(lock, source)
    out.mkdir()
    with tempfile.TemporaryDirectory(prefix="inspection-", dir=out) as temp:
        work = Path(temp)
        snapshot = work / image.name
        shutil.copyfile(image, snapshot)
        require(snapshot.stat().st_size == receipt["fileBytes"] and file_hash(snapshot) == receipt["sha256"], "final payload differs from build")
        snapshot.chmod(0o444)
        result = inspect_pair(Guest(snapshot, tools["guestfish"]), lock, receipt, tools, out, work)
        require(file_hash(snapshot) == receipt["sha256"] and file_hash(image) == receipt["sha256"], "payload changed during inspection")
    result.update(schemaVersion="bridge-guest-boot-assets/v1", architecture=lock["architecture"], diskSha256=receipt["sha256"],
                  buildReceiptSha256=hashlib.sha256(data).hexdigest(), inputSha256=receipt["inputSha256"],
                  extractorSha256=file_hash(Path(__file__)), inspectorSha256=file_hash(Path(__file__).with_name("boot_inspect.py")),
                  extractionTools=versions, publicationReady=False, bootVerified=False)
    pending = out / "boot-assets.json.partial"
    pending.write_bytes(encoded(result) + b"\n")
    pending.replace(out / "boot-assets.json")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lock", required=True, type=Path)
    parser.add_argument("--image", required=True, type=Path, help="final os.raw or os.qcow2, never a build intermediate")
    parser.add_argument("--build-receipt", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path, help="new boot-asset directory; failed attempts are retained")
    args = parser.parse_args()
    try:
        extract_boot_assets(load_lock(args.lock), args.image, args.build_receipt, args.out, source=args.lock.parent)
    except (ValueError, OSError, KeyError, TypeError, tarfile.TarError, subprocess.CalledProcessError) as error:
        print("guest boot assets: " + str(error), file=sys.stderr)
        return 2
    print("Boot assets verified against final guest bytes; no boot or publication approval.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
