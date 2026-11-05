"""Validate final guest boot exports; no guest execution or kernel derivation."""
import hashlib
import json
import uuid
from pathlib import Path

from images.guest import lock
from images.guest.boot_assets import initrd_compression, kernel_format
from images.guest.boot_inspect import DRIVERS, ENDPOINT_TOOLS

require, shape, matches = lock.require, lock.shape, lock.matches
TOOL_VERSION = r"[^\x00-\x08\x0b\x0c\x0e-\x1f\x7f]{1,4096}"
PROVENANCE = ("diskSha256", "bootReceiptSha256", "buildReceiptSha256", "inputSha256", "kernelConfigSha256",
              "modulesSha256", "endpointSha256", "earlyHookSha256", "buildHookSha256", "extractorSha256", "inspectorSha256")
DIRECT = ("kernel", "initrd", "rootSelector", "kernelRelease", "commandLine", "requiredDrivers", "provenance")


def tool_version(value):
    return matches(value, TOOL_VERSION) and bool(value.strip())


def regular(path):
    path = Path(path).absolute()
    require(not path.is_symlink() and path.is_file(), "missing or linked regular file")
    return path.resolve()


def measured(path, record):
    path = regular(path)
    require(path.stat().st_size == record["bytes"] and lock.file_hash(path) == record["sha256"], "boot asset bytes/checksum mismatch")
    return path


def command_line(selector, architecture):
    shape(selector, ("kind", "value"))
    require(selector["kind"] == "filesystem-uuid" and isinstance(selector["value"], str), "invalid root selector")
    require(str(uuid.UUID(selector["value"])) == selector["value"], "noncanonical root UUID")
    console = "hvc0" if architecture == "arm64" else "ttyS0,115200n8"
    return "root=UUID=" + selector["value"] + " ro console=" + console + " noresume"


def validate(direct, architecture, stem, tools):
    shape(direct, DIRECT)
    for role in ("kernel", "initrd"):
        record = direct[role]
        shape(record, ("name", "bytes", "sha256", "format", *(("compression",) if role == "initrd" else ())))
        require(record["name"] == stem + "." + role and type(record["bytes"]) is int and record["bytes"] > 0
                and matches(record["sha256"], lock.HEX), "invalid boot asset identity")
    require(direct["kernel"]["format"] == ("linux-arm64-image" if architecture == "arm64" else "linux-bzimage"), "wrong kernel format")
    require(direct["initrd"]["format"] == "initramfs" and direct["initrd"]["compression"] in ("gzip", "xz", "zstd", "cpio-prefix"), "invalid initrd format")
    require(matches(direct["kernelRelease"], r"[A-Za-z0-9._+-]{1,96}-" + architecture), "wrong kernel release architecture")
    require(direct["commandLine"] == command_line(direct["rootSelector"], architecture), "unsupported direct command line")
    drivers = direct["requiredDrivers"]
    require(isinstance(drivers, list) and len(drivers) == len(DRIVERS), "missing required drivers")
    for driver in drivers:
        shape(driver, ("name", "builtIn"))
        require(type(driver["builtIn"]) is bool, "invalid driver evidence")
    require({d["name"] for d in drivers} == set(DRIVERS), "wrong required drivers")
    shape(direct["provenance"], (*PROVENANCE, "extractionTools"))
    require(all(matches(direct["provenance"][k], lock.HEX) for k in PROVENANCE), "invalid boot provenance")
    extraction = direct["provenance"]["extractionTools"]
    require(isinstance(extraction, list) and len(extraction) == 4, "missing extraction tools")
    selected = {t["name"]: t for t in tools}
    require(selected.get("bridge-guest", {}).get("sha256") == direct["provenance"]["endpointSha256"], "endpoint component identity mismatch")
    for tool in extraction:
        shape(tool, ("name", "version", "executableSha256"))
        require(tool["name"] in selected and tool_version(tool["version"])
                and tool["version"] == selected[tool["name"]]["version"]
                and tool["executableSha256"] == selected[tool["name"]]["sha256"], "extraction tool identity mismatch")
    require({t["name"] for t in extraction} == {"guestfish", "unmkinitramfs", "readelf", "dpkg-query"}, "wrong extraction tool set")


def verify_files(direct, architecture, kernel, initrd):
    require(kernel_format(measured(kernel, direct["kernel"]), architecture) == direct["kernel"]["format"], "kernel format mismatch")
    require(initrd_compression(measured(initrd, direct["initrd"])) == direct["initrd"]["compression"], "initrd compression mismatch")


def consume(metadata, build_receipt, boot_receipt, kernel, initrd):
    # Parse and hash the same bounded bytes, preserving producer serialization.
    raw_build, raw_boot = {}, {}
    for path, holder in ((build_receipt, raw_build), (boot_receipt, raw_boot)):
        path = regular(path)
        require(path.stat().st_size <= 8 * 1024**2, "receipt too large")
        with path.open("rb") as stream:
            data = stream.read(8 * 1024**2 + 1)
        require(len(data) <= 8 * 1024**2, "receipt too large")
        holder.update(value=lock.read_json(data), sha256=hashlib.sha256(data).hexdigest())
    build, receipt = raw_build["value"], raw_boot["value"]
    shape(receipt, ("schemaVersion", "architecture", "diskSha256", "buildReceiptSha256", "inputSha256",
                    "kernel", "initrd", "rootSelector", "kernelRelease", "commandLine", "requiredDrivers",
                    "kernelConfigSha256", "modulesSha256", "endpointSha256", "earlyHookSha256", "buildHookSha256",
                    "extractorSha256", "inspectorSha256", "extractionTools", "initrdTools", "publicationReady", "bootVerified"))
    require(isinstance(build, dict) and build.get("schemaVersion") == "bridge-guest-build/v1"
            and receipt["schemaVersion"] == "bridge-guest-boot-assets/v1", "unsupported build/boot receipt schema")
    architecture = metadata["architecture"]
    require(build.get("architecture") == receipt["architecture"] == architecture
            and build.get("payload") == ("os.raw" if architecture == "arm64" else "os.qcow2")
            and build.get("bootVerified") is False and receipt["bootVerified"] is False
            and build.get("publicationReady") is False and receipt["publicationReady"] is False, "build/boot target or qualification mismatch")
    # The guest builder hashes compact sorted JSON without package.canonical's newline.
    inputs = hashlib.sha256(json.dumps(metadata["componentLock"], sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    require(build.get("inputSha256") == receipt["inputSha256"] == inputs
            and build.get("sha256") == receipt["diskSha256"]
            and receipt["buildReceiptSha256"] == raw_build["sha256"], "build/boot input or disk binding mismatch")
    require(type(build.get("fileBytes")) is int and build["fileBytes"] > 0, "invalid build payload size")
    tools = metadata["componentLock"]["tools"]
    selected = {t["name"]: t for t in tools}
    runtime = build.get("runtime", {})
    # build.py embeds the complete hash-bound producer record, including toolchain/ELF evidence.
    require(isinstance(runtime, dict), "invalid runtime provenance")
    require(isinstance(build.get("recipeFiles"), dict), "invalid build recipe provenance")
    require(runtime.get("schemaVersion") == "bridge-guest-binary/v1"
            and runtime.get("sourceCommit") == metadata["buildSourceCommit"]
            and all(selected.get(n, {}).get("version") == metadata["buildSourceCommit"] for n in ("bridge-guest", "bridge-guest-provenance"))
            and runtime.get("target") == ("aarch64-unknown-linux-musl" if architecture == "arm64" else "x86_64-unknown-linux-musl")
            and runtime.get("contractVersion") == "H1/v1" and type(runtime.get("protocolMajor")) is int and runtime["protocolMajor"] == 1,
            "runtime source/target provenance mismatch")
    require(runtime.get("binarySha256") == receipt["endpointSha256"] == selected["bridge-guest"]["sha256"], "endpoint provenance mismatch")
    for key, path in (("earlyHookSha256", "/etc/initramfs-tools/scripts/local-bottom/bridge-state"),
                      ("buildHookSha256", "/etc/initramfs-tools/hooks/bridge-state")):
        require(receipt[key] == build.get("recipeFiles", {}).get(path), "hook provenance mismatch")
    for key, name in (("extractorSha256", "boot_assets.py"), ("inspectorSha256", "boot_inspect.py")):
        require(receipt[key] == lock.file_hash(Path(lock.__file__).with_name(name)), "extractor/inspector provenance mismatch")
    inspected = receipt["initrdTools"]
    require(isinstance(inspected, list) and len(inspected) == len(ENDPOINT_TOOLS), "missing initrd tool inspection")
    for tool in inspected:
        shape(tool, ("initrdPath", "guestCanonicalPath", "bytes", "guestSha256", "initrdSha256", "initrdType", "dependencies"))
        require(isinstance(tool["dependencies"], list), "invalid initrd dependencies")
        for record in (tool, *tool["dependencies"]):
            shape(record, ("initrdPath", "guestCanonicalPath", "bytes", "guestSha256", "initrdSha256", "initrdType"), ("dependencies",))
            require(record["initrdType"] == "regular" and type(record["bytes"]) is int and record["bytes"] > 0
                    and matches(record["guestSha256"], lock.HEX) and record["guestSha256"] == record["initrdSha256"], "invalid initrd tool evidence")
    require({t["initrdPath"] for t in inspected} == set(ENDPOINT_TOOLS), "wrong initrd tool inspection set")
    stem = "-".join([metadata["imageId"], metadata["platform"], architecture])
    direct = {k: receipt[k] for k in ("rootSelector", "kernelRelease", "commandLine", "requiredDrivers")}
    direct["provenance"] = {**{k: receipt[k] for k in PROVENANCE if k != "bootReceiptSha256"}, "bootReceiptSha256": raw_boot["sha256"], "extractionTools": receipt["extractionTools"]}
    for role in ("kernel", "initrd"):
        require(receipt[role]["name"] == "os." + role, "wrong producer asset name")
        direct[role] = dict(receipt[role], name=stem + "." + role)
    validate(direct, architecture, stem, tools)
    verify_files(direct, architecture, kernel, initrd)
    return direct, build["fileBytes"]
