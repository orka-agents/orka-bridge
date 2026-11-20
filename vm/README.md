# Bridge VM prototype

A working prototype of Bridge's local environment: a Linux VM with a container runtime,
kind, a kind cluster running Orka, and kmx, plus a one-line installer that boots it with
QEMU/KVM on a Linux host.

The image pipeline Bridge will ship lives in [images/](../images/README.md): locked guest
inputs, the two-disk guest build with a separate state disk, its reference boot, the
guest endpoint and gated packaging. That pipeline does not install Orka yet. This
prototype does, on a single-disk Debian VM with the cluster created at build time, so
that Orka can be tried in a VM today and the Orka layer of that pipeline can build on
what it shows: the install path kmx adopts, the boot ordering Orka needs, and timings.

## What's in the image

| Component | Version | Notes |
|---|---|---|
| Base OS | Debian 13 generic cloud image | cloud-init runs once at build time, then is disabled |
| Container runtime | Debian `docker.io` | |
| kind | v0.33.0 | node image `kindest/node:v1.36.4`, pinned by digest |
| Orka | v0.2.0 | installed from its release chart; images pinned by digest |
| kmx | v0.9.0-rc.6 | `/usr/local/bin/kmx`, with kubectl and Helm |

Every other download is pinned in [versions.env](versions.env); the Debian packages come
from the signed Debian archive without version pins. The Orka chart checksum and image
digests come from the release's `candidate.json`, which is itself pinned by checksum.

Orka is installed the way `kmx cluster up` installs it: CRDs from the verified chart, then
`helm install orka` in `orka-system` with `fullnameOverride=orka-api`, harness v2, the
kmx ownership label and the release digests. The one addition is an `IfNotPresent` pull
policy for the controller, publisher and workers, so that boot doesn't need the registry;
kmx's adoption check reads the release, mode, name, label and digests, not pull policies.
kmx pins the Orka release it adopts (`kmx version` shows it), so an image for a newer Orka
needs a kmx release that targets that Orka. The build fails unless `kmx cluster status`
reports the running Orka as the release digest kmx pins and kmx's own
`kmx cluster orka install` keeps the release as it is ("already installed by kmx").

The cluster is created at build time and stopped before the disk is captured. The
controller, publisher and both worker images are already in the kind node. The four
coding-agent runtime images are pulled when first used; `PREPULL_ACP=1` adds them to the
image.

## Try it on Linux

You need x86_64 Linux, access to `/dev/kvm`, QEMU (`qemu-system-x86_64`, `qemu-img`) and
curl. Build an image and serve it locally:

```sh
sudo apt install qemu-system-x86 qemu-utils cloud-image-utils jq   # Debian/Ubuntu
BASE_URL=http://127.0.0.1:8765 vm/linux/build.sh /tmp/bridge-vm
python3 -m http.server 8765 --bind 127.0.0.1 --directory /tmp/bridge-vm
```

Then, in another terminal:

```sh
curl -fsSL http://127.0.0.1:8765/install.sh | bash
```

The installer puts `bridge-vm` in `~/.local/bin` (`BRIDGE_VM_BIN`), downloads the image,
checks its size and SHA-256 against the manifest, boots it and waits until the Orka
controller is ready and the Orka API answers `/readyz`. `BASE_URL` sets the address the
built `install.sh` downloads from; without it, `BRIDGE_VM_URL` is required when it runs.
It then prints the addresses and a kubeconfig path:

```text
Orka v0.2.0 is ready in the Bridge VM (…; kmx v0.9.0-rc.6 inside).

  Orka API and dashboard   http://127.0.0.1:18080
  Kubernetes API           https://127.0.0.1:16443
  kubeconfig               ~/.local/share/orka-bridge/vm/kubeconfig
```

The kubeconfig is separate from `~/.kube/config`, and its context is `orka-bridge-vm`.
Sign in to the dashboard with a token from
`kubectl -n orka-system create token orka-client`.

| Command | Does |
|---|---|
| `bridge-vm start` | boots the VM and waits for Orka |
| `bridge-vm stop` | shuts the VM down cleanly (ACPI power button; needs python3 or socat, otherwise it stops QEMU) |
| `bridge-vm status` | shows whether it is running and ready |
| `bridge-vm logs` | prints the VM console |
| `bridge-vm reset` | discards the VM's changes and returns to the downloaded image |
| `bridge-vm remove` | deletes the VM, the image, the kubeconfig and `bridge-vm` |

`BRIDGE_VM_CPUS`, `BRIDGE_VM_MEMORY` (MiB), `BRIDGE_VM_KUBE_PORT` and `BRIDGE_VM_ORKA_PORT`
override the defaults (up to 4 CPUs; 8 GiB of memory on hosts with 16 GB or more,
otherwise 4 GiB; ports 16443 and 18080).

## How it works

**Build** ([linux/build.sh](linux/build.sh)). The script boots the pinned Debian cloud image
once in QEMU, with a cloud-init seed that runs [guest/provision.sh](linux/guest/provision.sh).
That script installs Docker, kind, kubectl, Helm and kmx (each checked against its pinned
SHA-256), creates the `kaimahi-p1` kind cluster, installs Orka, pre-pulls the worker
images, stops the cluster, removes cloud-init state, SSH and apt timers, and powers off.
The disk is then written as a zstd-compressed qcow2 next to its manifest, `bridge-vm` and
`install.sh`.

**Boot** ([guest/bridge-vm-ready](linux/guest/bridge-vm-ready)). A systemd unit starts the kind
node and waits for the Kubernetes API and for Service addresses to work. Pods restart with
the node, some before that, and the controller exits at startup unless the
Workspace/Publisher answers. So the unit replaces the chart's publisher and proxy pods
that are not ready, waits for them, then does the same for the controller; Task and
worker pods are left alone. It then waits for the
Orka API to answer `/readyz` and the controller pod to be Ready. It writes its progress as
JSON lines to a virtio-serial port. The last line is `ready` (with the kubeconfig) or
`failed` (with a reason). `bridge-vm` reads that port from a file on the host. Nothing
listens for logins: the image has no SSH server and no passwords.

**Network.** QEMU user-mode networking forwards two ports, bound to the host's loopback
only: the Kubernetes API (guest 6443) and the Orka API (guest 8080, a kind port mapping to
the `orka-api-host` NodePort Service in front of the controller).

**State** lives in `~/.local/share/orka-bridge/vm` (`BRIDGE_VM_HOME`). The downloaded image
stays read-only; the VM writes to a qcow2 overlay on top of it, so `reset` is instant.

## Manifest

The build writes `manifest-linux-amd64.json`, a small prototype manifest that the
installer reads for the file name, size and checksums. Bridge's own image manifest is
[images/manifest.schema.json](../images/manifest.schema.json).

| Field | Example |
|---|---|
| `schemaVersion`, `platform` | `1`, `linux-amd64` |
| `orkaVersion`, `kmxVersion` | `v0.2.0`, `v0.9.0-rc.6` |
| `kindVersion`, `kubernetesVersion`, `baseImage` | `v0.33.0`, `v1.36.4`, `debian-13-genericcloud-amd64-…` |
| `imageFile`, `imageFormat`, `imageCompression` | `orka-bridge-vm-orka-v0.2.0-linux-amd64.qcow2`, `qcow2`, `zstd` |
| `imageSha256`, `imageSize` | checksum and size of the download |
| `imageDataSize`, `imageVirtualSize` | uncompressed data and disk size |
| `cliFile`, `cliSha256` | `bridge-vm` and its checksum |
| `created`, `sourceRevision`, `notes` | build time, commit, the kmx/Orka pairing |

## CI

[.github/workflows/vm-image.yml](../.github/workflows/vm-image.yml) runs the installer
tests, builds the image on a GitHub-hosted runner with KVM, installs it from a local
server, waits for Orka, stops and restarts it, and uploads the files as a short-lived
artifact. It publishes nothing; publishing belongs to the gated workflow in
[images/](../images/README.md).

## Measurements

On a 16-core x86_64 host with KVM, booting the VM with 4 vCPUs and 8 GiB, the download is
about 1.7 GB (4.2 GB uncompressed, on a 40 GiB virtual disk): about 2.2 minutes at
100 Mbit/s. The Kubernetes API is up 15–25 s after the VM starts. Across our runs, Orka
was ready 78–121 s after a cold boot and 77–121 s after a restart from a clean stop, which
takes about 12 s. Most of the spread is how often the Orka controller restarts while the
cluster settles (see below). The manifest records each image's exact size, and
`bridge-vm` prints how long Orka took to become ready.

## Limitations

- This is not the shipping layout. [images/](../images/README.md) defines a 16 GiB OS disk
  plus a host-created state disk, per-installation credentials, and the guest endpoint
  protocol on `ai.orka.bridge.control`. This prototype uses one disk, creates the cluster
  at build time, and reports readiness on its own port and format.
- Everything generated at build time is the same in every copy of the image: the cluster
  CA and service-account signing key, the kubeconfig, and Orka's webhook CA and snapshot
  encryption key. Both APIs listen only on the host's loopback, but any local user of the
  host can reach those ports.
- kubeadm issues the cluster's leaf certificates (API server, etcd, kubeconfig client) for
  one year, and nothing renews them at boot. An image stops working about a year after
  the `created` time in its manifest.
- The Orka controller restarts one to three times on every boot, first on Service
  addresses that kube-proxy has not programmed yet and then on a Workspace/Publisher
  connection that is not ready yet. The boot unit waits these out.
- Installing a different image does not migrate a VM: `bridge-vm remove` deletes its
  Tasks, Secrets and volumes first.
- kmx inside the VM cannot be run from the host. kmx on the host works against the VM with
  its kubeconfig for commands that only use the Kubernetes API.
- RHEL, Rocky and Alma Linux ship QEMU only as `/usr/libexec/qemu-kvm`, which `bridge-vm`
  does not use yet.
- macOS and Windows are not covered here; their image formats are part of the
  [images/](../images/README.md) contract.
