"""Verify engineering archives without extraction or executing their contents."""
import argparse
import hashlib
import json
import posixpath
import re
import stat
import struct
import sys
import zipfile
from pathlib import Path

import recipe

ARCHIVES = ("bridge-windows-runtime-portable.zip", "bridge-windows-runtime-source.zip")
MAX_ENTRIES, MAX_RUNTIME, MAX_SOURCE = 200000, 2 * 1024**3, 16 * 1024**3
EXES = {"bin/qemu-system-x86_64.exe", "bin/qemu-system-x86_64w.exe", "bin/qemu-img.exe"}
FEATURES = {**dict.fromkeys(("whpx", "slirp", "tools", "zstd", "pixman"), True),
            **dict.fromkeys(("tcg", "plugins", "sdl", "gtk", "vnc", "curses", "opengl", "virglrenderer",
                            "rutabaga_gfx", "libusb", "usb_redir", "virtfs", "guest_agent"), False),
            "audio_drv_list": []}
SYSTEM = {"kernel32", "kernelbase", "ntdll", "ucrtbase", "msvcrt", "advapi32", "ws2_32", "iphlpapi",
          "user32", "gdi32", "shell32", "shlwapi", "ole32", "oleaut32", "bcrypt", "crypt32", "secur32",
          "winmm", "winhvplatform", "version", "setupapi", "rpcrt4", "psapi", "comdlg32", "comctl32",
          "dbghelp", "dnsapi", "userenv", "netapi32", "wtsapi32", "imm32", "powrprof", "normaliz"}


def safe_path(name):
    if (not isinstance(name, str) or not name or name.startswith("/")
            or any(ord(c) < 32 or c in '\\<>:"|?*' for c in name)):
        raise ValueError(f"unsafe archive path: {name!r}")
    for part in name.split("/"):
        stem = part.split(".")[0].rstrip(" ").upper().translate(str.maketrans("¹²³", "123"))
        if (not part or part in (".", "..") or part.endswith((".", " "))
                or stem in recipe.RESERVED | {"CONIN$", "CONOUT$"}):
            raise ValueError(f"unsafe archive path: {name!r}")
    return name


def digest(stream, size, sink=None):
    sha, blob, count = hashlib.sha256(), hashlib.sha1(f"blob {size}\0".encode()), 0
    while chunk := stream.read(1024 * 1024):
        count += len(chunk)
        if count > size:
            raise ValueError("streamed member exceeds declared size")
        sha.update(chunk)
        blob.update(chunk)
        if sink is not None:
            sink.write(chunk)
    if count != size:
        raise ValueError("streamed member size mismatch")
    return sha.hexdigest(), blob.hexdigest()


def file_hash(path):
    with Path(path).open("rb") as stream:
        return digest(stream, Path(path).stat().st_size)[0]


def json_bytes(data):
    if len(data) > 64 * 1024**2:
        raise ValueError("JSON evidence exceeds limit")
    try:
        return json.loads(data, object_pairs_hook=recipe._unique_object)
    except (ValueError, UnicodeError, RecursionError) as error:
        raise ValueError(f"invalid JSON evidence: {error}") from error


def is_system(name):
    return (name.casefold().removesuffix(".dll") in SYSTEM and name.casefold().endswith(".dll")
            or bool(re.fullmatch(r"(?:api|ext)-ms-win-[a-z0-9-]+\.dll", name.casefold())))


def validate_links(links, names):
    def resolve(name, visited):
        parts = name.split("/")
        for i in range(1, len(parts) + 1):
            prefix = "/".join(parts[:i])
            if prefix in links:
                if prefix in visited:
                    raise ValueError(f"cyclic source link: {prefix}")
                target = links[prefix]
                if not target or target.startswith("/") or "\\" in target or ":" in target:
                    raise ValueError(f"unsafe source link: {prefix}")
                joined = posixpath.normpath(posixpath.join(posixpath.dirname(prefix), target, *parts[i:]))
                safe_path(joined)
                return resolve(joined, visited | {prefix})
        if name not in names and not any(p.startswith(name + "/") for p in names):
            raise ValueError(f"dangling source link: {name}")
    for name in links:
        try:
            resolve(name, set())
        except ValueError as error:
            raise ValueError(f"source link {name}: {error}") from error


def read_archive(path, source):
    limit = MAX_SOURCE if source else MAX_RUNTIME
    if path.stat().st_size > limit + MAX_ENTRIES * 1024:
        raise ValueError("archive compressed size exceeds limit")
    manifest_name = "provenance/source-manifest.json" if source else "provenance/runtime-manifest.json"
    records, folded, links, metadata, total = {}, set(), {}, {}, 0
    with zipfile.ZipFile(path) as archive:
        if len(archive.infolist()) > MAX_ENTRIES:
            raise ValueError("archive entry limit exceeded")
        for info in archive.infolist():
            name = safe_path(info.filename)
            if name.casefold() in folded:
                raise ValueError(f"duplicate/collision archive path: {name}")
            folded.add(name.casefold())
            mode = info.external_attr >> 16
            if (info.flag_bits & 1 or info.compress_type not in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED)
                    or not (stat.S_ISREG(mode) or source and stat.S_ISLNK(mode))):
                raise ValueError(f"unsupported archive member type: {name}")
            total += info.file_size
            if total > limit:
                raise ValueError("archive uncompressed limit exceeded")
            with archive.open(info) as stream:
                sha, blob = digest(stream, info.file_size)
            records[name] = {"path": name, "bytes": info.file_size, "sha256": sha, "mode": mode, "gitBlob": blob}
            if name.lower().endswith((".exe", ".dll")) and not source:
                with archive.open(info) as stream:
                    header = stream.read(65536)
                offset = struct.unpack_from("<I", header, 60)[0] if len(header) >= 64 else 65536
                if (header[:2] != b"MZ" or offset + 26 > len(header) or header[offset:offset+4] != b"PE\0\0"
                        or struct.unpack_from("<H", header, offset + 4)[0] != 0x8664
                        or struct.unpack_from("<H", header, offset + 24)[0] != 0x20b):
                    raise ValueError(f"not an AMD64 PE32+ image: {name}")
            if stat.S_ISLNK(mode):
                if info.file_size > 4096:
                    raise ValueError(f"source link target too long: {name}")
                links[name] = archive.read(info).decode("utf-8")
            if name.startswith("provenance/") and name.endswith(".json") or name == "features.txt":
                if info.file_size > 64 * 1024**2:
                    raise ValueError("JSON evidence exceeds limit")
                metadata[name] = json_bytes(archive.read(info))
        for name in records:
            for i in range(1, len(name.split("/"))):
                if "/".join(name.split("/")[:i]).casefold() in folded:
                    raise ValueError(f"file/directory path collision: {name}")
        validate_links(links, records)
        manifest = metadata.get(manifest_name)
        if not isinstance(manifest, dict) or not isinstance(manifest.get("files"), list):
            raise TypeError(f"missing manifest: {manifest_name}")
        expected = {}
        for row in manifest["files"]:
            name = safe_path(row["path"])
            if name in expected:
                raise ValueError(f"duplicate manifest entry: {name}")
            expected[name] = row
        actual = {k: v for k, v in records.items() if k != manifest_name}
        if expected != actual:
            raise ValueError("archive inventory differs from manifest")
    return records, metadata, manifest


def check_evidence(evidence, sources, packages, hashes):
    required = {"sourceCommit", "lockSha256", "installed", "imports", "firmware", "features", "tools"}
    if not isinstance(evidence, dict) or not required <= evidence.keys():
        raise ValueError("missing build evidence")
    if evidence["sourceCommit"] != sources["qemu"]["commit"] or evidence["lockSha256"] != hashes:
        raise ValueError("build evidence source/lock identity mismatch")
    recipe.check_installed(packages, evidence["installed"])
    if evidence["features"] != FEATURES or not evidence["tools"]:
        raise ValueError("effective headless features or tool evidence missing")
    if evidence["firmware"] != packages["firmware"] or not evidence["firmware"]:
        raise ValueError("firmware source map missing or differs from lock")


def git_tree(items):
    """Recreate Git's tree hash from relative paths, modes and object IDs."""
    root = {}
    for path, value in items.items():
        parts, node = safe_path(path).split("/"), root
        for part in parts[:-1]:
            node = node.setdefault(part, {})
            if not isinstance(node, dict):
                raise TypeError(f"Git path collision: {path}")
        if parts[-1] in node:
            raise ValueError(f"Git path collision: {path}")
        node[parts[-1]] = value
    def encode(node):
        entries = [(name, "40000", encode(value)) if isinstance(value, dict) else (name, *value)
                   for name, value in node.items()]
        entries.sort(key=lambda e: e[0].encode() + (b"/" if e[1] == "40000" else b""))
        body = b"".join(f"{mode} {name}\0".encode() + bytes.fromhex(oid) for name, mode, oid in entries)
        return hashlib.sha1(f"tree {len(body)}\0".encode() + body).hexdigest()
    return encode(root)


def check_git_index(rows, metadata, sources, packages):
    index = metadata.get("provenance/git-index.json")
    if not isinstance(index, dict):
        raise TypeError("missing Git index evidence")
    expected = {p["path"]: p["commit"] for p in recipe.source_repos(sources, packages)}
    actual = {}
    for p in index["repositories"]:
        name = safe_path(p["path"])
        if name in actual:
            raise ValueError("duplicate Git source repository")
        recipe._hex(p["tree"], 40)
        actual[name] = p["commit"]
    if actual != expected:
        raise ValueError("Git source repository identities differ from locks")
    paths = {p for p in rows if any(p.startswith(root + "/") for root in expected)}
    if set(index["files"]) != paths:
        raise ValueError("Git source inventory differs from archive")
    trees = {name: {} for name in expected}
    for name in paths:
        item, row = index["files"][name], rows[name]
        if (item["mode"] not in ("100644", "100755", "120000") or item["oid"] != row["gitBlob"]
                or stat.S_ISLNK(row["mode"]) and item["mode"] != "120000"):
            raise ValueError(f"Git blob/mode mismatch: {name}")
        owner = max((root for root in expected if name.startswith(root + "/")), key=len)
        trees[owner][name[len(owner)+1:]] = item["mode"], row["gitBlob"]
    for p in index["repositories"]:
        items = trees[p["path"]]
        if not isinstance(p["gitlinks"], dict):
            raise TypeError("invalid Git link inventory")
        for name, oid in p["gitlinks"].items():
            safe_path(name)
            recipe._hex(oid, 40)
            if name in items:
                raise ValueError("Git blob/gitlink collision")
            items[name] = "160000", oid
        tree = git_tree(items)
        if tree != packages["sourceTrees"][p["path"]] or tree != p["indexTree"]:
            raise ValueError(f"Git source tree differs from external lock: {p['path']}")


def verify(output, source_lock, packages_lock):
    try:
        output = Path(output)
        sources, packages = recipe.load_locks(source_lock, packages_lock)
        hashes = {"sources": file_hash(source_lock), "packages": file_hash(packages_lock)}
        expected_sums = "".join(f"{file_hash(output / name)}  {name}\n" for name in ARCHIVES)
        if (output / "SHA256SUMS").read_text(encoding="utf-8") != expected_sums:
            raise ValueError("archive SHA256SUMS mismatch")
        runtime, rm, manifest = read_archive(output / ARCHIVES[0], False)
        source, sm, source_manifest = read_archive(output / ARCHIVES[1], True)
        evidence = recipe.read_json(output / "build-report.json")
        check_evidence(evidence, sources, packages, hashes)
        check_git_index(source, sm, sources, packages)
        for rows, meta, header in ((runtime, rm, manifest), (source, sm, source_manifest)):
            if header.get("lockSha256") != hashes or header.get("sourceCommit") != evidence["sourceCommit"]:
                raise ValueError("manifest external lock mismatch")
            for name, lock in (("sources", sources), ("packages", packages)):
                member = f"provenance/{name}.lock.json"
                if meta.get(member) != lock or rows[member]["sha256"] != hashes[name]:
                    raise ValueError(f"embedded {name} lock differs from external lock")
        imports = evidence["imports"]
        pe_files = {p for p in runtime if p.lower().endswith((".exe", ".dll"))}
        if set(imports) != pe_files or not EXES <= pe_files or {p for p in pe_files if p.lower().endswith(".exe")} != EXES:
            raise ValueError("PE import evidence/executable inventory missing or extra")
        package_by_name = {p["name"]: p for p in packages["packages"]}
        reachable, owners = set(), set()
        def visit(name):
            if name in reachable:
                return
            reachable.add(name)
            for edge in imports[name]:
                if "/" in edge["name"] or "\\" in edge["name"]:
                    raise ValueError("invalid DLL basename")
                if edge["system"]:
                    if not is_system(edge["name"]):
                        raise ValueError(f"unknown system import: {edge['name']}")
                else:
                    target, owner = edge["path"], edge["owner"]
                    if (target not in pe_files or Path(target).name.casefold() != edge["name"].casefold()
                            or owner not in package_by_name or package_by_name[owner]["redistribution"] != "runtime"):
                        raise ValueError(f"missing DLL/package owner: {edge['name']}")
                    owners.add(owner)
                    visit(target)
        for name in EXES:
            visit(name)
        if (reachable != pe_files or rm.get("provenance/imports.json") != imports
                or rm.get("features.txt") != evidence["features"]):
            raise ValueError("unreachable DLL or mismatched import/feature evidence")
        for owner in owners:
            package = package_by_name[owner]
            entry = package["source"]
            row = source.get("inputs/" + entry["path"], {})
            if row.get("bytes") != entry["bytes"] or row.get("sha256") != entry["sha256"]:
                raise ValueError(f"missing/mismatched corresponding source: {owner}")
            notices = {r["sha256"] for p, r in runtime.items() if p.startswith(f"LICENSES/msys2/{owner}/") and r["bytes"]}
            if not {n["sha256"] for n in package["source"]["notices"]} <= notices:
                raise ValueError(f"missing/mismatched package license/notice: {owner}")
        for entry in recipe.source_inputs(sources, packages):
            row = source.get("inputs/" + entry["path"], {})
            if row.get("bytes") != entry["bytes"] or row.get("sha256") != entry["sha256"]:
                raise ValueError(f"missing source input: {entry['path']}")
        source_roots = {r["path"] for r in sm["provenance/git-index.json"]["repositories"]}
        for entry in packages["firmware"]:
            if (entry["path"] not in runtime or entry["license"] not in source or entry["source"] not in source_roots
                    or not entry["license"].startswith(entry["source"] + "/")):
                raise ValueError(f"missing firmware source/license: {entry['path']}")
        for notice in packages.get("firmwareNotices", []):
            for rows, name in ((source, notice["path"]), (runtime, "LICENSES/firmware/" + notice["path"])):
                row = rows.get(name, {})
                if row.get("bytes") != notice["bytes"] or row.get("sha256") != notice["sha256"]:
                    raise ValueError(f"missing/mismatched firmware notice: {name}")
        actual_firmware = {p for p in runtime if p.startswith("share/qemu/")}
        if actual_firmware != {p["path"] for p in packages["firmware"]}:
            raise ValueError("firmware inventory differs from source map")
        reference = packages["referenceNotice"]
        for rows, name in ((runtime, "LICENSES/reference/LICENSE"), (source, "inputs/" + reference["path"])):
            row = rows.get(name, {})
            if row.get("bytes") != reference["bytes"] or row.get("sha256") != reference["sha256"]:
                raise ValueError("missing/mismatched reference license")
        for name in ("recipe.py", "verify.py", "build.sh", "README.md", "windows-runtime.yml"):
            if not source.get("recipe/" + name, {}).get("bytes"):
                raise ValueError(f"missing rebuild recipe: {name}")
        for name, value in hashes.items():
            if source.get(f"recipe/{name}.lock.json", {}).get("sha256") != value:
                raise ValueError(f"missing/mismatched rebuild recipe lock: {name}")
        for name in ("COPYING", "COPYING.LIB", "LICENSE"):
            if f"LICENSES/qemu/{name}" not in runtime or f"qemu/{name}" not in source:
                raise ValueError(f"missing QEMU source/license: {name}")
        return {"publicationReady": False, "qualification": "build-only; no VM boot",
                "sourceCommit": evidence["sourceCommit"], "lockSha256": hashes,
                "runtimeFiles": len(runtime), "sourceFiles": len(source)}
    except (OSError, KeyError, TypeError, UnicodeError, ValueError, zipfile.BadZipFile, struct.error) as error:
        raise ValueError(f"archive verification: {error}") from error


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--sources", type=Path, required=True)
    parser.add_argument("--packages", type=Path, required=True)
    args = parser.parse_args()
    try:
        print(json.dumps(verify(args.output, args.sources, args.packages), sort_keys=True))
        return 0
    except ValueError as error:
        print(str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
