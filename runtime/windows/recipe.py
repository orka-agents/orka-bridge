"""Validate locked inputs and produce build-only Windows runtime archives."""
import argparse
import base64
import hashlib
import io
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import zipfile
from pathlib import Path
from urllib.parse import urlsplit

PATCHES = ("0001", "0005", "0009", "0015", "0019", "0024")
CHECKS = {"test-socket-close.py", "test-fd-setsize.py", "test-whpx-cpu-model.py"}
GROUPS = ("patches", "checks", "wraps")
RESERVED = {"CON", "PRN", "AUX", "NUL", *(f"{p}{n}" for p in ("COM", "LPT") for n in range(1, 10))}


def _keys(value, keys):
    if not isinstance(value, dict) or set(value) != set(keys.split()):
        raise ValueError(f"expected fields: {keys}")


def _hex(value, length):
    if not isinstance(value, str) or not re.fullmatch(rf"[0-9a-f]{{{length}}}", value):
        raise ValueError(f"expected lowercase {length}-character digest or commit")


def _path(value):
    if not isinstance(value, str) or not re.fullmatch(r"[A-Za-z0-9_+.~-]+(?:/[A-Za-z0-9_+.~-]+)*", value):
        raise ValueError(f"unsafe source path: {value!r}")
    for part in value.split("/"):
        if part in (".", "..") or part.endswith(".") or part.split(".")[0].upper() in RESERVED:
            raise ValueError(f"unsafe source path: {value!r}")
    return value


def _repository(value):
    if not isinstance(value, str):
        raise TypeError("expected HTTPS source repository")
    url = urlsplit(value)
    if (url.scheme != "https" or url.netloc not in ("github.com", "gitlab.com")
            or not url.path.strip("/") or url.query or url.fragment or any(c.isspace() for c in value)):
        raise ValueError(f"invalid source repository: {value!r}")


def _unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def load_sources(path):
    """Check the source-selection shape; commit existence is not proven here."""
    try:
        data = json.loads(Path(path).read_text(encoding="utf-8"), object_pairs_hook=_unique_object)
        _keys(data, "schemaVersion sourceDateEpoch reference qemu patches checks submodules wraps")
        if data["schemaVersion"] != "bridge-windows-sources/v1":
            raise ValueError("unsupported source lock schema")
        if type(data["sourceDateEpoch"]) is not int or data["sourceDateEpoch"] < 315532800:
            raise ValueError("invalid sourceDateEpoch")
        for name, keys in (("reference", "repository commit"), ("qemu", "repository commit baseCommit")):
            _keys(data[name], keys)
            _repository(data[name]["repository"])
            _hex(data[name]["commit"], 40)
        _hex(data["qemu"]["baseCommit"], 40)
        if (data["reference"]["repository"] != "https://github.com/omacom/try-omarchy-windows"
                or data["qemu"]["repository"] != "https://github.com/cmspam/winq-emu-qemu"):
            raise ValueError("unexpected source repository selection")
        origins = {
            "patches": f"https://raw.githubusercontent.com/omacom/try-omarchy-windows/{data['reference']['commit']}/runtime-build/",
            "checks": f"https://raw.githubusercontent.com/omacom/try-omarchy-windows/{data['reference']['commit']}/runtime-build/",
            "wraps": f"https://raw.githubusercontent.com/cmspam/winq-emu-qemu/{data['qemu']['commit']}/",
        }
        seen = set()
        for group in (*GROUPS, "submodules"):
            if not isinstance(data[group], list) or not data[group]:
                raise ValueError(f"missing source group: {group}")
            for item in data[group]:
                _keys(item, "path repository commit" if group == "submodules" else "path url bytes sha256")
                name = _path(item["path"])
                if name.casefold() in seen:
                    raise ValueError(f"duplicate source path: {name}")
                seen.add(name.casefold())
                if group == "submodules":
                    _repository(item["repository"])
                    _hex(item["commit"], 40)
                    continue
                if type(item["bytes"]) is not int or item["bytes"] <= 0:
                    raise ValueError(f"invalid byte length: {name}")
                _hex(item["sha256"], 64)
                relative = name.removeprefix("checks/") if group == "checks" else name
                if item["url"] != origins[group] + relative:
                    raise ValueError(f"source URL must match pinned origin and path: {name}")
                if group == "patches" and not (name.startswith("patches/qemu/") and name.endswith(".patch")):
                    raise ValueError(f"unexpected patch path: {name}")
                if group == "checks" and not name.startswith("checks/"):
                    raise ValueError(f"unexpected check path: {name}")
                if group == "wraps" and not (name.startswith("subprojects/") and name.endswith(".wrap")):
                    raise ValueError(f"unexpected wrap path: {name}")
        if tuple(Path(x["path"]).name[:4] for x in data["patches"]) != PATCHES:
            raise ValueError("patch selection or application order differs from headless profile")
        if sorted(Path(x["path"]).name for x in data["checks"]) != sorted(CHECKS):
            raise ValueError("required source checks are missing or substituted")
        return data
    except (OSError, UnicodeError, TypeError, ValueError) as error:
        raise ValueError(f"{path}: {error}") from error


def check_sources(lock, inputs):
    """Hash regular source files; inputs must remain stable for the whole check."""
    try:
        root = Path(inputs).resolve(strict=True)
        if not root.is_dir():
            raise ValueError("not a directory")
        count = 0
        for group in GROUPS:
            for item in lock[group]:
                path = root
                for part in _path(item["path"]).split("/"):
                    path = path / part
                    info = path.lstat()
                    if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & 0x400:
                        raise ValueError(f"link or reparse point: {item['path']}")
                if not stat.S_ISREG(info.st_mode):
                    raise ValueError(f"not a regular source file: {item['path']}")
                with path.open("rb") as stream:
                    size = 0
                    digest = hashlib.sha256()
                    while chunk := stream.read(1024 * 1024):
                        size += len(chunk)
                        if size > item["bytes"]:
                            raise ValueError(f"source byte length mismatch: {item['path']}")
                        digest.update(chunk)
                if size != item["bytes"] or digest.hexdigest() != item["sha256"]:
                    raise ValueError(f"source size/sha256 mismatch: {item['path']}")
                count += 1
        return count
    except (OSError, ValueError) as error:
        raise ValueError(f"source inputs {inputs}: {error}") from error


def read_json(path):
    try:
        return json.loads(Path(path).read_text(encoding="utf-8"), object_pairs_hook=_unique_object)
    except (OSError, UnicodeError, ValueError, RecursionError) as error:
        raise ValueError(f"{path}: {error}") from error


def inventory(rows):
    result = {}
    if not isinstance(rows, list) or not rows:
        raise ValueError("missing installed inventory")
    for row in rows:
        _keys(row, "name version")
        if (not isinstance(row["name"], str) or not re.fullmatch(r"[a-zA-Z0-9_.+-]+", row["name"])
                or not isinstance(row["version"], str) or not re.fullmatch(r"[a-zA-Z0-9_.+:~\-]+", row["version"])):
            raise ValueError("invalid package name/version")
        if row["name"] in result:
            raise ValueError(f"duplicate installed package: {row['name']}")
        result[row["name"]] = row["version"]
    return result


def captured_workflow(recipe_dir):
    """A source-ZIP rebuild keeps the workflow beside the recipe; the repository keeps it under .github."""
    beside = recipe_dir / "windows-runtime.yml"
    return beside if beside.is_file() else recipe_dir.parents[1] / ".github/workflows/windows-runtime.yml"


def package_inputs(packages):
    entries = [packages["bootstrap"], packages["referenceNotice"], *packages["packages"], *packages["pythonArchives"],
               *packages.get("firmwareRecipes", [])]
    entries += [p["source"] for p in packages["packages"] if "source" in p]
    result = {}
    for entry in entries:
        name = _path(entry["path"])
        identity = {key: entry[key] for key in ("path", "url", "bytes", "sha256")}
        if name.casefold() in result and result[name.casefold()] != identity:
            raise ValueError(f"conflicting input identities: {name}")
        result[name.casefold()] = identity
    return list(result.values())


def source_inputs(sources, packages):
    return ([p for group in GROUPS for p in sources[group]]
            + [p["source"] for p in packages["packages"] if "source" in p]
            + [packages["referenceNotice"], *packages["pythonArchives"], *packages.get("firmwareRecipes", [])])


def load_locks(source, packages):
    sources, data = load_sources(source), read_json(packages)
    try:
        if data["schemaVersion"] != "bridge-windows-packages/v1":
            raise ValueError("unsupported package schema")
        expected = inventory(data["bootstrapPackages"])
        selected = inventory([{"name": p["name"], "version": p["version"]} for p in data["packages"]])
        expected.update(selected)
        if inventory(data["expectedInstalled"]) != expected:
            raise ValueError("expected installed inventory omits or changes selected packages")
        for p in data["packages"]:
            if not p["license"] or p["redistribution"] not in ("build-only", "runtime"):
                raise ValueError(f"missing package license/selection: {p['name']}")
            if p["redistribution"] == "runtime":
                if "source" not in p:
                    raise ValueError(f"missing corresponding source: {p['name']}")
                source_data = p["source"]
                if source_data["version"] != p["version"]:
                    raise ValueError(f"corresponding source version differs: {p['name']}")
                names = {Path(m["path"]).name for m in source_data["recipeMembers"]}
                if not {"PKGBUILD", ".SRCINFO"} <= names or not source_data["sourceReferences"] or not source_data["notices"]:
                    raise ValueError(f"missing source recipe/distfile/notice: {p['name']}")
                for ref in source_data["sourceReferences"]:
                    if ref["present"] is not True or not ref["archiveMembers"]:
                        raise ValueError(f"missing source distfile: {p['name']}")
                members = [*source_data["recipeMembers"], *source_data["notices"]]
                members += [m for ref in source_data["sourceReferences"] for m in ref["archiveMembers"]]
                for member in members:
                    _hex(member["sha256"], 64)
                    if type(member["bytes"]) is not int or member["bytes"] < 0:
                        raise ValueError(f"invalid source member identity: {p['name']}")
        entries = [data["bootstrap"], data["referenceNotice"], *data["packages"], *data["pythonArchives"], *data.get("firmwareRecipes", [])]
        entries += [p["source"] for p in data["packages"] if "source" in p]
        for entry in entries:
            _path(entry["path"])
            _hex(entry["sha256"], 64)
            if type(entry["bytes"]) is not int or entry["bytes"] <= 0:
                raise ValueError(f"invalid archive byte length: {entry['path']}")
            for url in (entry["url"], entry["provenance"]["url"]):
                value = urlsplit(url)
                if (value.scheme != "https" or value.netloc not in
                        ("repo.msys2.org", "github.com", "api.github.com", "raw.githubusercontent.com")
                        or value.username or value.query or value.fragment):
                    raise ValueError(f"unexpected input origin: {entry['path']}")
            _hex(entry["provenance"]["sha256"], 64)
            if not isinstance(entry["provenance"]["method"], str) or not entry["provenance"]["method"]:
                raise ValueError("missing input provenance method")
        for root in data.get("extraSources", []):
            _path(root["path"])
            if not root["path"].startswith(("firmware/", "qemu/subprojects/")):
                raise ValueError("unexpected extra Git source destination")
            for entry in [root, *root.get("submodules", [])]:
                _path(entry["path"])
                _repository(entry["repository"])
                _hex(entry["commit"], 40)
        if "policyPatch" in data:
            policy = data["policyPatch"]
            if policy["path"] != "disable-dsound.patch" or type(policy["bytes"]) is not int or policy["bytes"] <= 0:
                raise ValueError("invalid owned policy patch")
            _hex(policy["sha256"], 64)
        if "patchedQemuTree" in data:
            _hex(data["patchedQemuTree"], 40)
        package_inputs(data)
        roots = {r["path"] for r in source_repos(sources, data)}
        if not isinstance(data["sourceTrees"], dict) or set(data["sourceTrees"]) != roots:
            raise ValueError("missing/unexpected Git source tree pins")
        for tree in data["sourceTrees"].values():
            _hex(tree, 40)
        return sources, data
    except (KeyError, TypeError, ValueError) as error:
        raise ValueError(f"{packages}: {error}") from error


def check_installed(packages, rows):
    if inventory(rows) != inventory(packages["expectedInstalled"]):
        raise ValueError("installed package inventory differs from lock")


def check_inputs(sources, packages, inputs):
    check_sources(sources, inputs)
    check_sources({"patches": package_inputs(packages), "checks": [], "wraps": []}, inputs)


def import_names(text):
    if "file format pei-x86-64" not in text:
        raise ValueError("objdump did not identify AMD64 PE")
    names = [line.split("DLL Name:", 1)[1].strip() for line in text.splitlines() if "DLL Name:" in line]
    if any(not re.fullmatch(r"[A-Za-z0-9_.+-]+\.dll", name, re.IGNORECASE) for name in names):
        raise ValueError("invalid PE import basename")
    return names


def dll_table(paths):
    result = {}
    for path in paths:
        name = path.name.casefold()
        if name in result:
            raise ValueError(f"conflicting DLL basename: {name}")
        result[name] = path
    return result


def resolve_import(name, candidates):
    import verify
    if verify.is_system(name):
        return None
    if name.casefold() not in candidates:
        raise ValueError(f"unresolved non-system DLL: {name}")
    return candidates[name.casefold()]


def output(args):
    return subprocess.check_output([str(a) for a in args], text=True, encoding="utf-8")


def source_repos(sources, packages):
    rows = [{"path": "qemu", **sources["qemu"]}]
    rows += [{**p, "path": "qemu/" + p["path"]} for p in sources["submodules"]]
    for root in packages.get("extraSources", []):
        rows.append({k: root[k] for k in ("path", "repository", "commit")})
        rows += [{"path": root["path"] + "/" + p["path"], "repository": p["repository"], "commit": p["commit"]}
                 for p in root.get("submodules", [])]
    if len({r["path"].casefold() for r in rows}) != len(rows):
        raise ValueError("duplicate Git checkout path")
    return rows


def snapshot(checkouts, destination, rows):
    """Export only indexed blob bytes, retaining original modes without links."""
    import verify
    destination = Path(destination)
    destination.mkdir()
    expected = {r["path"]: r["commit"] for r in rows}
    files, repositories = {}, []
    for row in rows:
        root = Path(checkouts) / row["path"]
        git = ["git", "-C", root]
        if output([*git, "rev-parse", "HEAD"]).strip() != row["commit"]:
            raise ValueError(f"Git checkout identity differs: {row['path']}")
        output([*git, "diff-files", "--quiet"])
        metadata = {"path": row["path"], "commit": row["commit"], "gitlinks": {},
                    "tree": output([*git, "rev-parse", "HEAD^{tree}"]).strip(),
                    "indexTree": output([*git, "write-tree"]).strip()}
        for entry in output([*git, "ls-files", "--stage", "-z"]).split("\0"):
            if not entry:
                continue
            fields, name = entry.split("\t", 1)
            mode, oid, stage = fields.split()
            full = verify.safe_path(row["path"] + "/" + name)
            if stage != "0":
                raise ValueError("unmerged Git index")
            if mode == "160000":
                metadata["gitlinks"][name] = oid
                if full in expected and expected[full] != oid:
                    raise ValueError(f"Git submodule commit differs: {full}")
                continue
            path = root / name
            info = path.lstat()
            if not stat.S_ISREG(info.st_mode) or getattr(info, "st_file_attributes", 0) & 0x400:
                raise ValueError(f"source is not a Git blob placeholder: {full}")
            data = path.read_bytes()
            if hashlib.sha1(f"blob {len(data)}\0".encode() + data).hexdigest() != oid:
                raise ValueError(f"Git blob bytes differ: {full}")
            target = destination / full
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
            target.chmod(0o755 if mode == "100755" else 0o644)
            representation = "file" if mode != "120000" else "content-placeholder" if b"\n" in data else "link-placeholder"
            files[full] = {"mode": mode, "oid": oid, "representation": representation}
        repositories.append(metadata)
    (destination / "provenance").mkdir()
    (destination / "provenance/git-index.json").write_text(json.dumps({"files": files, "repositories": repositories}, sort_keys=True) + "\n", encoding="utf-8")


def effective_features(options):
    import verify
    result = {}
    for key, expected in verify.FEATURES.items():
        value = options.get(key)
        if isinstance(value, str) and value in ("enabled", "disabled"):
            value = value == "enabled"
        if type(value) is not type(expected):
            raise ValueError(f"missing/unknown effective feature state: {key}")
        result[key] = value
    return result


def check_compiled_features(configs):
    required = ("WHPX", "SLIRP", "ZSTD", "PIXMAN")
    forbidden = ("TCG", "PLUGIN", *(f"AUDIO_{k}" for k in ("ALSA", "COREAUDIO", "DSOUND", "JACK", "OSS", "PA", "PIPEWIRE", "SDL", "SNDIO")))
    for key in (*required, *forbidden):
        values = {v.strip() for v in re.findall(rf"^#define CONFIG_{key}(?:[ \t]+(.*))?$", configs, re.MULTILINE)}
        states = {v != "0" for v in values}
        if values - {"", "0", "1"} or len(states) > 1:
            raise ValueError(f"unknown/conflicting compiled feature: {key}")
        if (True in states) != (key in required):
            reason = "compiled feature missing" if key in required else "forbidden compiled feature"
            raise ValueError(f"{reason}: {key}")


def collect(args, locks):
    """Stage selected executables and their full locked non-system DLL closure."""
    import verify
    sources, packages = load_locks(*locks)
    installed = [dict(zip(("name", "version"), line.split())) for line in output(["pacman", "-Q"]).splitlines()]
    check_installed(packages, installed)
    options = {r["name"]: r["value"] for r in json.loads(output([args.meson, "introspect", "--buildoptions", args.build]))}
    features = effective_features(options)
    if (features != verify.FEATURES or options["auto_features"] != "disabled" or not options["install_blobs"]
            or not options["default_devices"] or options["rust"] != "disabled"):
        raise ValueError("effective Meson feature profile differs from headless selection")
    for key in ("alsa", "coreaudio", "dsound", "jack", "oss", "pa", "pipewire", "sndio", "spice", "dbus_display"):
        if options[key] != "disabled":
            raise ValueError(f"host backend remains enabled: {key}")
    check_compiled_features("\n".join(p.read_text() for p in args.build.glob("*config*.h")))
    runtime = args.runtime
    (runtime / "bin").mkdir(parents=True)
    for name in sorted(verify.EXES):
        matches = list(args.install.rglob(Path(name).name))
        if len(matches) != 1:
            raise ValueError(f"missing/ambiguous installed executable: {name}")
        shutil.copyfile(matches[0], runtime / name)
    blobs = list(args.install.rglob("bios-256k.bin"))
    if len(blobs) != 1:
        raise ValueError("missing/ambiguous firmware data root")
    (runtime / "share").mkdir()
    shutil.copytree(blobs[0].parent, runtime / "share/qemu", symlinks=True)
    candidates = dll_table([p for p in Path("/ucrt64/bin").iterdir() if p.suffix.lower() == ".dll"])
    declared = {p["name"]: p for p in packages["packages"]}
    imports, owners, pending = {}, set(), sorted(verify.EXES)
    while pending:
        name = pending.pop()
        if name in imports:
            continue
        imports[name] = []
        for dll in import_names(output(["objdump", "-p", runtime / name])):
            path = resolve_import(dll, candidates)
            edge = {"name": dll, "system": path is None}
            if path is not None:
                owner = output(["pacman", "-Qoq", path]).strip()
                if owner not in declared or declared[owner]["redistribution"] != "runtime":
                    raise ValueError(f"DLL source/notice owner is not locked: {dll}: {owner}")
                target = "bin/" + path.name
                edge.update(path=target, owner=owner)
                owners.add(owner)
                if not (runtime / target).exists():
                    shutil.copyfile(path, runtime / target)
                pending.append(target)
            imports[name].append(edge)
    snapshot(args.checkouts, args.source, source_repos(sources, packages))
    for entry in source_inputs(sources, packages):
        path = args.source / "inputs" / entry["path"]
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(args.inputs / entry["path"], path)
    for name in ("recipe.py", "verify.py", "build.sh", "README.md", "disable-dsound.patch"):
        path = args.source / "recipe" / name
        path.parent.mkdir(exist_ok=True)
        shutil.copyfile(Path(__file__).with_name(name), path)
    shutil.copyfile(captured_workflow(Path(__file__).resolve().parent), args.source / "recipe/windows-runtime.yml")
    for name, lock in zip(("sources", "packages"), locks):
        shutil.copyfile(lock, args.source / f"recipe/{name}.lock.json")
    (runtime / "LICENSES/reference").mkdir(parents=True)
    shutil.copyfile(args.inputs / packages["referenceNotice"]["path"], runtime / "LICENSES/reference/LICENSE")
    for name in ("COPYING", "COPYING.LIB", "LICENSE"):
        path = runtime / "LICENSES/qemu" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(args.source / "qemu" / name, path)
    for owner in sorted(owners):
        for notice in declared[owner]["source"]["notices"]:
            data = base64.b64decode(packages["noticeData"][notice["sha256"]], validate=True)
            if len(data) != notice["bytes"] or hashlib.sha256(data).hexdigest() != notice["sha256"]:
                raise ValueError(f"package notice bytes differ: {owner}")
            path = runtime / "LICENSES/msys2" / owner / (notice["sha256"] + ".txt")
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
    for notice in packages["firmwareNotices"]:
        data = (args.source / notice["path"]).read_bytes()
        if len(data) != notice["bytes"] or hashlib.sha256(data).hexdigest() != notice["sha256"]:
            raise ValueError(f"firmware notice differs: {notice['path']}")
        path = runtime / "LICENSES/firmware" / notice["path"]
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    evidence = {"sourceCommit": sources["qemu"]["commit"],
                "lockSha256": dict(zip(("sources", "packages"), (verify.file_hash(p) for p in locks))),
                "installed": installed, "imports": imports, "firmware": packages["firmware"], "features": features,
                "tools": {"compiler": output(["gcc", "--version"]).strip(), "meson": output([args.meson, "--version"]).strip(),
                          "msys": output(["uname", "-r"]).strip(), "runner": os.environ["BRIDGE_RUNNER_IMAGE"]}}
    args.evidence.write_text(json.dumps(evidence, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    return evidence


def package(runtime, source, output, locks, evidence):
    """Publish a new local directory only after both temporary archives verify."""
    import verify
    sources, packages = load_locks(*locks)
    hashes = dict(zip(("sources", "packages"), (verify.file_hash(p) for p in locks)))
    verify.check_evidence(evidence, sources, packages, hashes)
    output = Path(output)
    if output.exists():
        raise ValueError(f"output already exists: {output}")
    def encoded(value):
        return (json.dumps(value, sort_keys=True, indent=2) + "\n").encode()
    def walk(root):
        for path in sorted(root.iterdir()):
            mode = path.lstat()
            if getattr(mode, "st_file_attributes", 0) & 0x400 and not stat.S_ISLNK(mode.st_mode):
                raise ValueError(f"unexpected reparse point: {path.name}")
            if stat.S_ISDIR(mode.st_mode):
                yield from walk(path)
            elif stat.S_ISREG(mode.st_mode) or stat.S_ISLNK(mode.st_mode):
                yield path
            else:
                raise ValueError(f"not a regular file: {path.name}")
    with tempfile.TemporaryDirectory(prefix=".bridge-runtime-", dir=output.parent) as temp:
        staging = Path(temp) / "output"
        staging.mkdir()
        for index, root in enumerate((Path(runtime), Path(source))):
            records = []
            with zipfile.ZipFile(staging / verify.ARCHIVES[index], "w", compression=zipfile.ZIP_DEFLATED) as archive:
                def put(name, mode, size, reader):
                    verify.safe_path(name)
                    info = zipfile.ZipInfo(name, time.gmtime(sources["sourceDateEpoch"])[:6])
                    info.create_system, info.external_attr = 3, mode << 16
                    info.compress_type = zipfile.ZIP_DEFLATED
                    with archive.open(info, "w", force_zip64=True) as sink:
                        sha, blob = verify.digest(reader, size, sink)
                    return {"path": name, "bytes": size, "sha256": sha, "mode": mode, "gitBlob": blob}
                def data(name, value):
                    return put(name, stat.S_IFREG | 0o644, len(value), io.BytesIO(value))
                for path in walk(root):
                    name = path.relative_to(root).as_posix()
                    if path.is_symlink():
                        if not index:
                            raise ValueError(f"runtime link: {name}")
                        value = os.readlink(path).encode("utf-8")
                        records.append(put(name, stat.S_IFLNK | 0o777, len(value), io.BytesIO(value)))
                    else:
                        mode = stat.S_IFREG | (0o755 if path.stat().st_mode & 0o111 else 0o644)
                        with path.open("rb") as reader:
                            records.append(put(name, mode, path.stat().st_size, reader))
                for name, lock in zip(("sources", "packages"), locks):
                    records.append(data(f"provenance/{name}.lock.json", Path(lock).read_bytes()))
                if not index:
                    records.append(data("provenance/imports.json", encoded(evidence["imports"])))
                    records.append(data("features.txt", encoded(evidence["features"])))
                manifest = {"sourceCommit": evidence["sourceCommit"], "lockSha256": hashes,
                            "files": sorted(records, key=lambda x: x["path"])}
                data("provenance/" + ("source" if index else "runtime") + "-manifest.json", encoded(manifest))
        (staging / "build-report.json").write_bytes(encoded(evidence))
        (staging / "SHA256SUMS").write_text("".join(f"{verify.file_hash(staging / n)}  {n}\n" for n in verify.ARCHIVES), encoding="utf-8")
        result = verify.verify(staging, *locks)
        (staging / "verification.json").write_bytes(encoded(result))
        if output.exists():
            raise ValueError("output appeared during packaging")
        staging.rename(output)
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    check = commands.add_parser("check-sources", help="validate selection and optionally staged source bytes")
    check.add_argument("--lock", required=True, type=Path)
    check.add_argument("--inputs", type=Path)
    fields = {"check-locks": (), "list-inputs": (), "list-git": (), "check-inputs": ("inputs",),
              "check-installed": ("inventory",), "package": ("runtime", "source", "output", "evidence"),
              "stage": ("checkouts", "inputs", "build", "install", "runtime", "source", "evidence", "meson")}
    for name, options in fields.items():
        command = commands.add_parser(name)
        for field in ("packages", *(() if name == "check-installed" else ("sources",)), *options):
            command.add_argument("--" + field, required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        if args.command == "check-sources":
            lock = load_sources(args.lock)
            count = check_sources(lock, args.inputs) if args.inputs is not None else 0
            result = {"schemaVersion": "bridge-windows-source-check/v1", "publicationReady": False,
                      "qualification": "source-selection-only; no build or patch application",
                      "inputBytesVerified": args.inputs is not None, "filesVerified": count,
                      "qemuCommit": lock["qemu"]["commit"]}
        elif args.command == "check-installed":
            check_installed(read_json(args.packages), read_json(args.inventory))
            result = {"publicationReady": False, "qualification": "installed-inventory-only"}
        else:
            locks = args.sources, args.packages
            sources, packages = load_locks(*locks)
            if args.command == "list-git":
                for row in sorted(source_repos(sources, packages), key=lambda r: (r["path"].count("/"), r["path"])):
                    print("\t".join(row.get(k, "-") for k in ("path", "repository", "commit", "baseCommit")))
                return 0
            entries = [p for group in GROUPS for p in sources[group]] + package_inputs(packages)
            result = {"publicationReady": False, "qualification": "input-selection-only", "inputFiles": len(entries),
                      "expectedInstalled": len(packages["expectedInstalled"])}
            if args.command == "list-inputs":
                result = entries
            elif args.command == "check-inputs":
                check_inputs(sources, packages, args.inputs)
                result["inputBytesVerified"] = True
            elif args.command == "package":
                result = package(args.runtime, args.source, args.output, locks, read_json(args.evidence))
            elif args.command == "stage":
                collect(args, locks)
                result["qualification"] = "staged-only; no archive or VM qualification"
        print(json.dumps(result, sort_keys=True))
        return 0
    except (ValueError, OSError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        print(f"{args.command}: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
