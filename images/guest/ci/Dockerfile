# Builder for the guest CI reference boot. Run it with --network none and
# --device /dev/kvm; it holds no inputs, credentials or network fallbacks.
# Packages come from the current trixie mirror on the digest-pinned base.
# Each lock records the builder tool hashes and exact versions it used, and
# each boot receipt records the QEMU version and firmware hashes.
FROM debian:trixie-20261027@sha256:2dc419c6d7fe3dad0a2f9fd469c34cc90b0f9ba642d8fde837fe3d7d34a0e7f1
RUN apt-get -o Acquire::Retries=3 update \
 && DEBIAN_FRONTEND=noninteractive apt-get -o Acquire::Retries=3 install -y --no-install-recommends \
      genisoimage libguestfs-tools linux-image-amd64 ovmf python3 qemu-system-x86 qemu-utils \
 && rm -rf /var/lib/apt/lists/*
ENV LIBGUESTFS_BACKEND=direct PYTHONDONTWRITEBYTECODE=1
