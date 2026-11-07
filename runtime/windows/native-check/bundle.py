"""Assemble the bounded native-check bundle from an authenticated producer download."""
import argparse
import json
import re
import shutil
import stat
import sys
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import verify

SOURCE_HEAD = "7bec6ae15764680c887c7103b9c62d8b3fab3a52"
PRODUCER_HEAD = "006b30f02a3c9cec61f8f3cc32fde6b0d248fc34"
PRODUCER_RUN = 42120608528
PRODUCER_FILES = (*verify.ARCHIVES, "SHA256SUMS", "build-report.json", "verification.json")
BUNDLE_FILES = (*PRODUCER_FILES, "ci-state.log", "ci-state-receipt.json", "ci-state-stderr.txt", "bridge-native-premises.exe", "runtime-pin.json",
                "producer-run.json", "bundle-provenance.json", "independent-verification.json",
                "build.log", "probe-dependents.txt", "producer-watch.log",
                "guard-test-build.log", "ci-guard-tests.txt", "ci-guard-tests-stderr.txt",
                "ci-stdout.txt", "ci-stderr.txt", "ci-controller.json",
                "ci-owned-pipe-stdout.txt", "ci-owned-pipe-stderr.txt",
                "ci-af-warmup.json", "ci-af-control.json", "ci-af-private.json", "ci-af-client-stderr.txt",
                "ci-qmp-rx.jsonl", "ci-qmp-tx.jsonl", "ci-qmp-disconnect-rx.jsonl",
                "ci-qmp-disconnect-tx.jsonl", "ci-qmp-summary.json")


def write_json(path, value):
    path.write_text(json.dumps(value, sort_keys=True, indent=2) + "\n", encoding="utf-8")


def regular(path):
    mode = path.lstat()
    if not stat.S_ISREG(mode.st_mode) or getattr(mode, "st_file_attributes", 0) & 0x400:
        raise ValueError(f"not a regular, non-reparse file: {path.name}")


def prepare(producer, run_file, sources, packages, exe, output, runtime, driver_head):
    producer, run_file, exe, output, runtime = map(Path, (producer, run_file, exe, output, runtime))
    # This record must come from the workflow's authenticated, literal gh run view.
    # A JSON file and self-described hashes alone are not authentication.
    run = json.loads(run_file.read_text(encoding="utf-8-sig"))
    expected = {"databaseId": PRODUCER_RUN, "headSha": PRODUCER_HEAD, "headBranch": "main",
                "event": "workflow_dispatch", "workflowName": "Windows runtime",
                "status": "completed", "conclusion": "success"}
    if not isinstance(run, dict) or any(run.get(key) != value for key, value in expected.items()):
        raise ValueError("wrong or unsuccessful producer run")
    if not re.fullmatch(r"[0-9a-f]{40}", driver_head):
        raise ValueError("invalid driver commit")
    if runtime.exists() or runtime.is_symlink():
        raise FileExistsError(f"runtime destination already exists: {runtime}")
    for path in (exe, run_file, *(producer / name for name in PRODUCER_FILES)):
        regular(path)
    verified = verify.verify(producer, sources, packages)
    records, _, _ = verify.read_archive(producer / verify.ARCHIVES[0], False)
    pin = {
        "head": SOURCE_HEAD, "producerRun": PRODUCER_RUN, "producerHead": PRODUCER_HEAD,
        "runtimeZipSha256": verify.file_hash(producer / verify.ARCHIVES[0]),
        "runtimeFiles": [{"name": name, "bytes": row["bytes"], "sha256": row["sha256"]}
                         for name, row in sorted(records.items())],
    }
    # The verifier rejects aliases, traversal, links, special files and collisions.
    # Inputs stay admin-owned and stable until the later standard-user step.
    runtime.mkdir()
    with zipfile.ZipFile(producer / verify.ARCHIVES[0]) as archive:
        archive.extractall(runtime, members=sorted(records))
    output.mkdir(exist_ok=True)
    for name in PRODUCER_FILES:
        shutil.copyfile(producer / name, output / name)
    shutil.copyfile(exe, output / "bridge-native-premises.exe")
    shutil.copyfile(run_file, output / "producer-run.json")
    write_json(output / "runtime-pin.json", pin)
    write_json(output / "independent-verification.json", verified)
    write_json(output / "bundle-provenance.json", {
        "driverHead": driver_head, "sourceHead": SOURCE_HEAD,
        "producerRun": PRODUCER_RUN, "producerHead": PRODUCER_HEAD,
        "toolchain": "1.98.1-x86_64-pc-windows-msvc", "rustflags": "-Ctarget-feature=+crt-static",
        "executableSha256": verify.file_hash(exe), "runtimeZipSha256": pin["runtimeZipSha256"],
        "qualification": "no VM; native result is in ci-stdout.txt, not this metadata",
        "guestAssets": "inert-not-qualified", "whpxUsability": "not-tested",
    })
    return pin


def finish(output):
    """Hash only explicit operator files that actually exist, including failed-run diagnostics."""
    output = Path(output)
    rows = []
    for name in sorted(BUNDLE_FILES):
        path = output / name
        if not path.exists() and not path.is_symlink():
            continue
        regular(path)
        rows.append(f"{verify.file_hash(path)}  {name}\n")
    if not rows:
        raise ValueError("no operator files to hash")
    (output / "operator-sha256sums.txt").write_text("".join(rows), encoding="utf-8")
    print("".join(rows), end="")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    prepare_parser = commands.add_parser("prepare")
    for name in ("producer", "run-file", "sources", "packages", "exe", "output", "runtime"):
        prepare_parser.add_argument("--" + name, type=Path, required=True)
    prepare_parser.add_argument("--driver-head", required=True)
    finish_parser = commands.add_parser("finish")
    finish_parser.add_argument("--output", type=Path, required=True)
    args = vars(parser.parse_args())
    command = args.pop("command")
    try:
        if command == "prepare":
            prepare(**args)
        else:
            finish(**args)
        return 0
    except (OSError, ValueError, zipfile.BadZipFile) as error:
        print(f"native bundle: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
