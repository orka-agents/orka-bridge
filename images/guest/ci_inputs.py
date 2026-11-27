"""Fetch and lock the CI reference-boot inputs; never a release input set."""
import argparse
import base64
import binascii
import hashlib
import http.client
import json
import shutil
import subprocess
import sys
import time
import urllib.request
from pathlib import Path
from typing import TYPE_CHECKING

if __package__ or TYPE_CHECKING:
    from .build import run, verify_os_inputs
    from .lock import HEX, NAME, ROLES, VERSIONS, checked_file, file_hash, matches, read_json, relative_path, require, shape
else:
    from build import run, verify_os_inputs
    from lock import HEX, NAME, ROLES, VERSIONS, checked_file, file_hash, matches, read_json, relative_path, require, shape

SOURCES = Path(__file__).with_name("ci") / "sources.json"
USES = {"package", "tool", "chart-fixture"}
BUILDER_TOOLS = ("qemu-img", "guestfish", "virt-resize", "virt-customize")
CHUNK = 1024 * 1024


def load_sources(path):
    value = read_json(path.read_bytes())
    shape(value, ("schemaVersion", "architecture", "files"))
    require(value["schemaVersion"] == "bridge-guest-ci-sources/v1", "unsupported CI sources schema")
    require(value["architecture"] == "amd64", "the CI reference boot is amd64 only")
    files = value["files"]
    require(isinstance(files, list) and files, "missing CI sources")
    names, paths = set(), []
    for row in files:
        shape(row, ("use", "name", "version", "path", "url", "bytes", "sha256"))
        require(row["use"] in USES, "unknown CI source use")
        require(matches(row["name"], NAME) and (row["use"], row["name"]) not in names, "invalid or duplicate CI source name")
        names.add((row["use"], row["name"]))
        require(isinstance(row["version"], str) and row["version"], "missing CI source version")
        require(isinstance(row["url"], str) and row["url"].startswith("https://"), "CI sources must use HTTPS")
        require(type(row["bytes"]) is int and row["bytes"] > 0, "invalid CI source size")
        require(matches(row["sha256"], HEX), "invalid CI source checksum")
        path = relative_path(row["path"])
        require(all(path != p and p not in path.parents and path not in p.parents for p in paths), "overlapping CI source paths")
        paths.append(path)
    fixtures = [row for row in files if row["use"] == "chart-fixture"]
    require(len(fixtures) == 1 and fixtures[0]["version"] == VERSIONS["sourceOfflineProfile"]["kmxSourceCommit"],
            "the chart fixture must come from the pinned kmx commit")
    return value


def owned_target(root, value):
    """A cached directory must not redirect writes through a symlink."""
    path = root
    for part in relative_path(value).parts:
        path = path / part
        require(not path.is_symlink(), "CI source symlink is not allowed")
    return path


def verified(path, row):
    return path.is_file() and path.stat().st_size == row["bytes"] and file_hash(path) == row["sha256"]


def download(row, target, opener):
    partial = target.with_name(target.name + ".partial")
    # A leftover partial, including a symlink, is removed; exclusive creation never follows one.
    partial.unlink(missing_ok=True)
    digest, size = hashlib.sha256(), 0
    try:
        with opener(row["url"], timeout=60) as response, partial.open("xb") as stream:
            while chunk := response.read(CHUNK):
                size += len(chunk)
                require(size <= row["bytes"], "CI source exceeds its pinned size: " + row["path"])
                digest.update(chunk)
                stream.write(chunk)
        require(size == row["bytes"] and digest.hexdigest() == row["sha256"], "CI source checksum mismatch: " + row["path"])
        partial.replace(target)
    finally:
        partial.unlink(missing_ok=True)


def fetch(sources, out, *, opener=urllib.request.urlopen, attempts=3, pause=time.sleep):
    """Networked step: keep verified files, replace anything else from its pinned URL."""
    # The output root is restored from a cache; resolve() below would hide a symlink there.
    require(not out.is_symlink(), "CI source symlink is not allowed")
    out.mkdir(parents=True, exist_ok=True)
    root = out.resolve()
    for row in sources["files"]:
        target = owned_target(root, row["path"])
        target.parent.mkdir(parents=True, exist_ok=True)
        start = time.monotonic()
        if verified(target, row):
            print("kept " + row["path"])
            continue
        # A present file that failed verification, such as a damaged cache entry, is reported as replaced.
        action = "replaced" if target.exists() else "fetched"
        for attempt in range(1, attempts + 1):
            try:
                download(row, target, opener)
                break
            except (OSError, ValueError, http.client.HTTPException) as error:
                if attempt == attempts:
                    raise ValueError("could not fetch " + row["path"] + ": " + str(error)) from error
                pause(10 * attempt)
        print("%s %s (%d bytes, %.1f s)" % (action, row["path"], row["bytes"], time.monotonic() - start))


def placeholder(role):
    """The base case stages no application archives; these rows name no files."""
    def digest(kind):
        return hashlib.sha256(("bridge-ci-unused/" + role + "/" + kind).encode()).hexdigest()
    return {"name": role, "repository": "localhost/bridge-ci-unused/" + role, "manifestDigest": "sha256:" + digest("manifest"),
            "configDigest": "sha256:" + digest("config"), "archive": "unused/" + role + ".oci.tar", "archiveSha256": digest("archive")}


def retained_chart(fixture):
    value = read_json(fixture.read_bytes())
    chunks = value.get("chartChunks") if isinstance(value, dict) else None
    if not isinstance(chunks, list) or not chunks or not all(isinstance(c, str) for c in chunks):
        raise ValueError("fixture has no retained chart")
    try:
        data = b"".join(base64.b64decode(chunk, validate=True) for chunk in chunks)
    except binascii.Error as error:
        raise ValueError("invalid retained chart encoding") from error
    require(hashlib.sha256(data).hexdigest() == VERSIONS["sourceOfflineProfile"]["chartSha256"], "retained chart differs from the pinned profile")
    return data


def copy_verified(source, target, expected):
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)
    require(file_hash(target) == expected, "input changed during copy: " + target.name)
    return expected


def assemble(sources, downloads, runtime, out):
    """Offline step inside the builder: copy verified inputs and pin its own tools."""
    require(not out.exists() and not out.is_symlink(), "output must not already exist")
    provenance = read_json((runtime / "provenance.json").read_bytes())
    require(isinstance(provenance, dict) and matches(provenance.get("sourceCommit"), r"[0-9a-f]{40}"), "endpoint provenance has no source commit")
    out.mkdir()
    selected = {"package": [], "tool": []}
    for row in sources["files"]:
        source = checked_file(downloads, row["path"], row["sha256"])
        if row["use"] == "chart-fixture":
            (out / "chart").mkdir()
            (out / "chart/orka.tgz").write_bytes(retained_chart(source))
            continue
        copy_verified(source, out / row["path"], row["sha256"])
        selected[row["use"]].append({key: row[key] for key in ("name", "version", "path", "sha256")})
    tools = selected["tool"]
    for name, file, dest in (("bridge-guest", "bridge-guest", "tools/bridge-guest"),
                             ("bridge-guest-provenance", "provenance.json", "tools/bridge-guest-provenance.json")):
        source = runtime / file
        require(source.is_file() and not source.is_symlink(), "missing endpoint build output: " + file)
        digest = copy_verified(source, out / dest, file_hash(source))
        tools.append({"name": name, "version": provenance["sourceCommit"], "path": dest, "sha256": digest})
    for name in BUILDER_TOOLS:
        found = shutil.which(name)
        if found is None:
            raise ValueError("missing builder tool: " + name)
        digest = copy_verified(found, out / "tools" / name, file_hash(Path(found)))
        tools.append({"name": name, "version": run([name, "--version"]), "path": "tools/" + name, "sha256": digest})
    components = [placeholder(role) for role in sorted(ROLES)]
    node = next(c for c in components if c["name"] == "kind-node")
    architecture = sources["architecture"]
    lock = {"schemaVersion": "bridge-guest-inputs/v1", "architecture": architecture, "sourceCommit": VERSIONS["orkaSource"],
            "orka": {"mode": "source", "identity": VERSIONS["orkaSource"], "chartPath": "chart/orka.tgz",
                     "chartSha256": VERSIONS["sourceOfflineProfile"]["chartSha256"]},
            "components": components, "packages": selected["package"], "tools": tools,
            "nodeLocalRef": "kindest/node:bridge-" + node["manifestDigest"][7:23] + "-" + architecture, "kmxProfileSha256": None}
    # The same lock, provenance and ELF checks that assembly applies.
    verify_os_inputs(lock, source=out)
    (out / "inputs.json").write_text(json.dumps(lock, indent=2) + "\n")
    return lock


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    fetch_parser = commands.add_parser("fetch", help="download and verify pinned files (networked)")
    fetch_parser.add_argument("--out", type=Path, required=True)
    lock_parser = commands.add_parser("lock", help="write a base-case input lock (offline, inside the builder)")
    lock_parser.add_argument("--downloads", type=Path, required=True)
    lock_parser.add_argument("--runtime", type=Path, required=True, help="directory with bridge-guest and provenance.json")
    lock_parser.add_argument("--out", type=Path, required=True)
    for command in (fetch_parser, lock_parser):
        command.add_argument("--sources", type=Path, default=SOURCES)
    args = parser.parse_args()
    try:
        sources = load_sources(args.sources)
        if args.command == "fetch":
            fetch(sources, args.out)
        else:
            assemble(sources, args.downloads, args.runtime, args.out.absolute())
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print("guest ci inputs: " + str(error), file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
