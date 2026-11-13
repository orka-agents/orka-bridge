# Image assets

Bridge image bytes stay separate from the desktop app. These standard-library
Python tools package one supplied OS disk, verify its chunks, and define a gated
manual workflow for **draft** release assets. They do not build a disk or VM.
The packaging tools require only Python 3.11 or newer. Schema regression tests
also use the development-only `jsonschema` validator (`python3-jsonschema` on
Ubuntu), installed before CI enters the network-disabled test step.

**`bridge-image/v2` is frozen.** `publisher-policy.json` accepts the exact schema
hash also pinned by the native consumer. Acceptance is recorded outside the schema.
Its paired source-preview profiles select retained image bytes, not a shipping
release. Neither caller-supplied metadata nor a successful fixture test approves
publication or proves a physical host walk. The shipping app catalog stays empty.

## Local packaging and verification

With a final unbooted guest disk, its unchanged `build.json`, the matching
`boot-assets.json` (`bridge-guest-boot-assets/v1`), `os.kernel`, `os.initrd`
and owner-supplied metadata:

```sh
python3 images/package.py --input os.qcow2 --metadata metadata.json \
  --build-receipt build.json --boot-receipt boot/boot-assets.json \
  --kernel boot/os.kernel --initrd boot/os.initrd --out package
python3 images/package.py --verify package/manifest.json
# Optional local reconstruction; the destination must not exist:
python3 images/package.py --verify package/manifest.json --out restored.disk
```

The default chunk limit is 1 GiB. `--chunk-bytes` accepts positive values up to
that limit for small tests through the same code. Packaging reads at most 1 MiB
at a time and creates deterministic gzip bytes within the same Python/zlib
toolchain. The manifest records ordered chunk names, byte counts and SHA-256,
the whole gzip identity, and measured original byte count and hash. These are
integrity checks against selected bytes, not signatures or source authenticity.

The output directory must be new. A completed package contains only disk chunks,
`<imageId>-<platform>-<architecture>.kernel`, the matching `.initrd`, and
`manifest.json`, finalized after all files verify. Raw receipts are provenance
inputs, not additional release assets. Interrupted output can remain partial;
choose a fresh directory, not an overwrite. Verification rejects missing, extra,
symbolically linked, reordered or damaged chunks or boot files, invalid gzip,
unsupported boot formats and mismatched measurements.
`--manifest-sha256` anchors verification to an independently selected manifest
hash. Without that option, verification checks internal consistency only.
Hash-only verification is the default and needs no second raw disk copy.
Optional reconstructed output must be outside the package directory and is
finalized without replacing an existing file.
A failed reconstruction leaves `<destination>.partial` for inspection; retry with
a fresh destination, rather than automatically deleting or trusting partial bytes.

## Input and manifest mapping

Metadata has exactly these fields: `imageId`, `buildSourceCommit`, `platform`,
`architecture`, `format`, `minimumHostVersion`, `minimumAppVersion`,
`stateCompatibilityId`, `transportProtocolMajor` and `componentLock`.
The last field is the existing [guest input lock](guest/README.md), not a new
producer provenance format. Paths in that input lock are never copied to the
public manifest. The builder commit is distinct from the lock's Orka commit.

Supported pairs are `macos/arm64/raw` and `windows/amd64/qcow2`. Linux is a guest
and reference-test platform, not an added desktop download target. Minimum host
and app versions are owner-supplied compatibility declarations, not measured
memory requirements or proof that a host can boot the guest.

V2 requires `defaultLaunch=linux-direct` on Windows and `defaultLaunch=uefi`
on Mac. Both publish the matching standalone pair. `layout.osDisk.boot=uefi`
still describes disk capability, not host launch selection; canonical H1 layout
validation is unchanged. The v2 validator explicitly rejects v1 without upgrading
it. The exact v1 schema and descriptors remain historical at commit
`4b810c232c627f2ccb1dd59456a7ddd415d2912d`.

The manifest binds the joint two-disk contract, `H1/v1`:

- The only downloadable disk is `os-disk`: a GPT/UEFI factory with 16 GiB
  virtual capacity. Each installation uses its own writable copy, identified as
  `bridge-os`; its later modified hash is not the factory download hash.
- `state-disk` is only a `create-empty` descriptor, with 32 GiB virtual capacity,
  whole-device ext4 and identifier `bridge-state`. The host creates it privately
  using the target format. `new-only` describes the policy, not permission to
  format an arbitrary blank or existing disk; live transaction authorization is
  still required. No initialized state payload is packaged.
- Devices bind virtio disks, NAT, fresh entropy, UTC, the private nonconsole port
  `ai.orka.bridge.control` and separate diagnostics. The transport major is 1
  with JSON-line framing. H1 also requires early persistent identity and volatile
  credentials before container services; this tool does not implement that boot.
- `stateCompatibilityId` comes from the owners. Four vCPU and 6 GiB guest RAM
  are engineering test settings. Private installation UUIDs, MACs, boot nonces,
  credential identifiers and host paths are not release-manifest fields.

Declared virtual capacity is separate from measured payload file bytes. Tiny
fixtures can carry the contract while remaining explicitly **non-bootable**;
these tools do not parse GPT, EFI or QCOW2 to certify an actual disk.

`components` records the guest lock's semantic input identity, canonical lock
byte hash and exact checked-in version-policy hash as separate identities.
Kubernetes is explicitly **policy-declared**, not covered solely by the semantic
lock hash. Orka, kind, Vekil and selected tools retain their available identities.
kmx is currently represented by its nullable profile checksum; this does not
invent an executable version/hash or assert that the guest endpoint exists.
`publicationReady` stays false, including after any separately approved preview
publication. See [the accepted schema](manifest.schema.json).

## Direct-boot input contract

`directBoot` contains kernel and initrd name/positive bytes/SHA-256 records,
`rootSelector` (canonical filesystem UUID), `kernelRelease`, `commandLine`,
`requiredDrivers` and `provenance` (including `extractionTools`). Kernel formats are
`linux-bzimage` (AMD64) or `linux-arm64-image` (ARM64). Initrd format is
`initramfs`; measured compression is `gzip`, `xz`, `zstd` or `cpio-prefix`
(the last may precede a later compressed archive). No kernel transformation or
free-form command-line override is supported. The exact derived strings are:

```text
root=UUID=<measured-uuid> ro console=ttyS0,115200n8 noresume
root=UUID=<measured-uuid> ro console=hvc0 noresume
```

Provenance binds `diskSha256`, `bootReceiptSha256`, `buildReceiptSha256`,
`inputSha256`, `kernelConfigSha256`, `modulesSha256`, `endpointSha256`,
`earlyHookSha256`, `buildHookSha256`, `extractorSha256` and `inspectorSha256`.
Extraction tools record exact names, raw bounded version text (including spaces
and newlines) and `executableSha256`, cross-checked against component tool pins.
Non-tool version policies are unchanged. The endpoint source must match
`buildSourceCommit` and both endpoint tool versions, not the Orka source commit.
Input SHA uses the guest builder's compact sorted lock JSON **without a newline**, distinct
from the manifest's newline-terminated `canonicalLockSha256` and semantic
`inputIdentity`. Extractor and inspector hashes must match the packaging source.

The Python API is `create(stream, metadata, out, chunk_bytes=MAX_CHUNK, *,
build_receipt, boot_receipt, kernel, initrd)`; the four new inputs are filesystem
paths. The disk stream must be seekable: bounded preflight hashing verifies the
final disk/build/boot binding before creating output, then the unchanged
compression/chunk loop hashes the consumed bytes again. Boot files must be
regular and unlinked, match the receipt and supported headers, and verify again
after copying. Verification checks the exact asset set before reconstruction.
These checks validate producer evidence and bytes, not a second guest inspection
or authenticity of a caller-invented receipt. Independent receipt/profile pins
and later native qualification remain necessary.

## Gated draft workflow

The checked-in profile must first pin the accepted schema bytes, repository,
workflow, successful build run, source commit, artifact ZIP hash and selected
payload, metadata, build receipt, boot receipt, kernel and initrd member hashes,
target, input identity and reviewed native-receipt identity. Profile fields are
`payloadPath/Sha256`, `metadataPath/Sha256`, `buildReceiptPath/Sha256`,
`bootReceiptPath/Sha256`, `kernelPath/Sha256/Bytes` and `initrdPath/Sha256/Bytes`
(each slash denotes separate fields). All six selected paths must differ. No producer member names or qualified build workflow are guessed here.
A profile is a reviewed repository change, not workflow input that self-approves.
Before network access, its architecture/input identity must match a reviewed
upstream release profile or the policy's `sourcePreviews` entry for the exact Orka
source commit. The publisher alone passes that table as keyword-only
`source_previews` to guest admission. Source previews retain the complete source
and offline-profile validation; `lock.py --mode release` still refuses them.
Release-mode locks still require a reviewed upstream release including #833.

The paired profiles retain locally built disks through input-only drafts. Their
`artifactId/artifactSha256` identify the successful **endpoint Actions artifact**,
not the disk ZIP. Run, workflow, source, nonfork repository, artifact digest and
artifact/run bindings remain checked. Only artifact expiry is ignored for this
retained-input path; Actions disk inputs still require an unexpired artifact.

`inputDraft` pins `releaseId`, `tag`, `archiveBytes`, `archiveSha256`, and ordered
`parts` (`name`, `bytes`, `sha256`). Input tags use
`bridge-image-inputs-<architecture>-<archiveSha256>`, separate from output image tags.
Exactly one matching unpublished draft must target the producer commit, with the
exact reviewed part inventory. Parts download by authenticated asset ID, verify
individually, and reconstruct a hash-pinned ZIP before the same six-member checks.
Draft identity and inventory are rechecked after download. ZIP64 permits the 16 GiB
Mac member without materializing another raw disk. Part files are removed from
private temporary space as each is joined; original receipts remain unchanged.
The historical `nativeReceiptSha256` field pins supporting verification evidence;
it is not interpreted by this tool and does not establish a native app walk.

The manually selected profile is checked on trusted `main`. The existing Actions
prepare job has read permissions only. Input drafts require push-capable GitHub
authentication; these profiles are exercised with local `prepare`, not claimed to
work in that read-only job. Workflow input acquisition is a separate integration
step; no permission expansion is implicit in these profiles. It first requires the builder commit's checked-in guest version
policy to match the publisher's exact policy bytes, so policy-declared Kubernetes
cannot silently refer to a different build policy. It validates run and artifact
identities, cross-binds metadata
to the same source and target, validates all selected member hashes and boot
sizes, and supplies the selected receipts and boot files to packaging. ZIP path,
duplicate and regular-file safety checks apply to every member, not only selected
files. Only receipts and boot assets are materialized in private temporary space;
the selected disk remains streamed. It never extracts a raw disk or runs downloaded code. Plan for
ZIP plus compressed-package space; the draft roundtrip needs two compressed
package copies. Available space and real input sizes must be checked before
using a qualified runner; a standard runner is not promised to fit every image.

Only the separate draft job has release-write permission. It requalifies the
same bytes against prepare's manifest hash, serializes the image's derived tag,
rejects existing tags and all matching releases including drafts, and checks its
new draft is unique. Any tag appearing during the operation must point directly
to the qualified source commit; other tag identities fail closed. Uploads never
clobber. It redownloads through authenticated
asset IDs and verifies the exact kernel/initrd/chunks/manifest list and every
file's bytes and hashes against the original manifest. The CLI always reruns
prepare with the approved profile before draft and compares the regenerated
manifest to prepare's externally pinned hash before any release creation. The
internal draft preflight also compares payload/build/boot hashes and both boot
assets' hashes and sizes to the selected profile. No extra public profile
attestation field is added. Failure leaves the
draft for operator inspection, never silently retries or publishes it. An input
draft must never be published. An output draft is not release approval: native
walks and a separate publication decision remain required. Schema acceptance does
not authorize dispatch or publication.

## Tests

```sh
python3 -m unittest discover -s images -p 'test_*.py' -v
python3 images/package.py --help
python3 images/qualify.py --help
actionlint .github/workflows/images.yml
```

Tests use small non-bootable payloads and synthetic ZIPs. GitHub API calls are
mocked at that boundary, including every draft mutation; no tests publish.
CI runs unit tests without external networking. Actual builder artifact download,
draft API mutation/roundtrip and native boot are not proved by these fixtures.
