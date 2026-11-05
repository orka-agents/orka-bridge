"""Read initramfs files as data; never execute guest tools on the builder."""
import hashlib
import lzma
import re
import stat
import tarfile
from collections import deque
from pathlib import PurePosixPath

from .lock import file_hash, require

ENDPOINT_TOOLS = ("/sbin/blkid", "/sbin/blockdev", "/sbin/mkfs.ext4", "/bin/mount", "/usr/bin/findmnt")
DRIVERS = {"virtio_pci": "VIRTIO_PCI", "virtio_blk": "VIRTIO_BLK", "virtio_console": "VIRTIO_CONSOLE",
           "virtio_rng": "HW_RANDOM_VIRTIO", "virtio_net": "VIRTIO_NET", "ext4": "EXT4_FS"}


def rooted_file(root, name, *, regular_leaf=False, directory=False, executable=False):
    """Resolve usr-merge links inside the archive root, including absolute links."""
    require(name.startswith("/"), "absolute initrd path required")
    pending, parts, links = deque(PurePosixPath(name).parts[1:]), [], 0
    while pending:
        part = pending.popleft()
        if part == "..":
            require(bool(parts), "initrd path escapes root")
            parts.pop()
            continue
        if part == ".":
            continue
        path = root.joinpath(*parts, part)
        mode = path.lstat().st_mode
        if stat.S_ISLNK(mode):
            require(not (regular_leaf and not pending), "initrd tool must be regular")
            links += 1
            require(links <= 40, "initrd symlink loop")
            target = PurePosixPath(path.readlink().as_posix())
            if target.is_absolute():
                parts = []
            pending.extendleft(reversed(target.parts[1:] if target.is_absolute() else target.parts))
        else:
            parts.append(part)
    result = root.joinpath(*parts)
    mode = result.lstat().st_mode
    require(stat.S_ISDIR(mode) if directory else stat.S_ISREG(mode), "initrd file must be regular or selected directory")
    require(not executable or mode & 0o111 != 0, "initrd boot file must be executable")
    return result


def module_identity(archive):
    rows = []
    with tarfile.open(archive, "r:") as tar:
        for member in tar:
            if not member.isfile():
                continue  # In particular, do not follow build/source links out of the selected tree.
            name = member.name.removeprefix("./")
            require(not name.startswith("/") and ".." not in name.split("/"), "invalid module path")
            stream = tar.extractfile(member)
            if stream is None:
                raise ValueError("missing module bytes")
            digest = hashlib.sha256()
            with stream:
                while data := stream.read(1024**2):
                    digest.update(data)
            rows.append((name, member.size, digest.hexdigest()))
    require(rows and len({r[0] for r in rows}) == len(rows), "missing or duplicate module files")
    require({"modules.dep", "modules.builtin"} <= {r[0] for r in rows}, "missing module metadata")
    return rows


MAX_MODULE_BYTES = 128 * 1024**2


def module_hashes(archive, rows, requested):
    """Keep physical identities; compare XZ modules expanded by initramfs-tools."""
    hashes = {name: digest for name, _, digest in rows}
    needed = {name + ".xz" for name in requested if name.endswith(".ko") and name not in hashes}
    if not needed:
        return hashes
    with tarfile.open(archive, "r:") as tar:
        for member in tar:
            name = member.name.removeprefix("./")
            if name not in needed or not member.isfile():
                continue
            require(member.size <= MAX_MODULE_BYTES, "compressed module exceeds bound")
            stream = tar.extractfile(member)
            if stream is None:
                raise ValueError("missing compressed module bytes")
            with stream:
                compressed = stream.read(MAX_MODULE_BYTES + 1)
            require(len(compressed) <= MAX_MODULE_BYTES, "compressed module exceeds bound")
            try:
                decoder = lzma.LZMADecompressor(memlimit=256 * 1024**2)
                data = decoder.decompress(compressed, max_length=MAX_MODULE_BYTES + 1)
            except lzma.LZMAError as error:
                raise ValueError("invalid XZ kernel module") from error
            require(len(data) <= MAX_MODULE_BYTES and decoder.eof and not decoder.unused_data,
                    "incomplete, trailing or oversized XZ kernel module")
            hashes[name.removesuffix(".xz")] = hashlib.sha256(data).hexdigest()
    return hashes


def dependencies(root, executable, architecture, readelf, run):
    """Inspect ELF dependency names and verify their transitive initrd files."""
    triplet = "x86_64-linux-gnu" if architecture == "amd64" else "aarch64-linux-gnu"
    directories = ("/lib/" + triplet, "/usr/lib/" + triplet, "/lib64", "/usr/lib64", "/lib", "/usr/lib")
    queue, seen, found = [executable], set(), {}
    while queue:
        current = queue.pop()
        if current in seen:
            continue
        seen.add(current)
        data = run([readelf, "-l", "-d", current], env={"LC_ALL": "C"})
        require(not re.search(r"\((?:RPATH|RUNPATH)\)", data), "unsupported dependency search path")
        names = re.findall(r"Shared library: \[([^\]]+)\]", data)
        interpreters = re.findall(r"Requesting program interpreter: ([^\]]+)\]", data)
        for name in [*interpreters, *names]:
            require(name.startswith("/") or re.fullmatch(r"[A-Za-z0-9._+-]+", name), "invalid dependency name")
            candidates = [name] if name.startswith("/") else [d + "/" + name for d in directories]
            for path in candidates:
                try:
                    file = rooted_file(root, path, executable=name in interpreters)
                except FileNotFoundError:
                    continue
                if file not in found:
                    found[file] = path
                    queue.append(file)
                break
            else:
                raise ValueError("missing initrd dependency: " + name)
    return sorted(found.values())


def compare_guest_file(guest, root, path, scratch, *, regular_leaf=False, executable=False):
    installed = rooted_file(root, path, regular_leaf=regular_leaf, executable=executable)
    canonical = guest.call("realpath", path)
    require(canonical.startswith("/") and ".." not in canonical.split("/"), "invalid canonical guest path")
    guest.call("download", canonical, scratch)
    digest = file_hash(installed)
    require(digest == file_hash(scratch), "initrd file bytes differ from guest: " + path)
    return {"initrdPath": path, "guestCanonicalPath": canonical, "bytes": installed.stat().st_size,
            "guestSha256": digest, "initrdSha256": digest, "initrdType": "regular"}
