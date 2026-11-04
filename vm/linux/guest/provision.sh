#!/bin/bash
# Runs once inside the build VM (as root, from cloud-init) and leaves a prepared kind
# cluster with Orka installed. The build host reads the console for the final marker.
set -euo pipefail

# shellcheck source=/dev/null
. /opt/bridge/build.env

export DEBIAN_FRONTEND=noninteractive
export HOME=/root
export KUBECONFIG=/etc/bridge-vm/kubeconfig
export KIND_CLUSTER=kaimahi-p1

CLUSTER=kaimahi-p1
CONTEXT=kind-$CLUSTER
NODE=$CLUSTER-control-plane
NAMESPACE=orka-system

log() { printf '[bridge-provision] %s\n' "$*"; }

fetch() { # url sha256 destination
  local tmp
  tmp=$(mktemp /opt/bridge/fetch.XXXXXX)
  curl -fsSL --retry 5 --retry-all-errors -o "$tmp" "$1"
  if ! printf '%s  %s\n' "$2" "$tmp" | sha256sum -c --quiet -; then
    log "checksum mismatch for $1"
    exit 1
  fi
  mv "$tmp" "$3"
}

log "packages"
# Background apt runs would hold the dpkg lock and later change the image under the user.
systemctl disable --now apt-daily.timer apt-daily-upgrade.timer unattended-upgrades.service 2>/dev/null || true
apt-get -o DPkg::Lock::Timeout=600 update -q
apt-get -o DPkg::Lock::Timeout=600 install -y -q --no-install-recommends docker.io docker-cli ca-certificates curl
systemctl enable --now docker.service

log "binaries"
fetch "https://github.com/kubernetes-sigs/kind/releases/download/$KIND_VERSION/kind-linux-amd64" \
  "$KIND_LINUX_AMD64_SHA256" /usr/local/bin/kind
fetch "https://dl.k8s.io/release/$KUBECTL_VERSION/bin/linux/amd64/kubectl" \
  "$KUBECTL_LINUX_AMD64_SHA256" /usr/local/bin/kubectl
fetch "https://get.helm.sh/helm-$HELM_VERSION-linux-amd64.tar.gz" \
  "$HELM_LINUX_AMD64_SHA256" /opt/bridge/helm.tgz
tar -xzf /opt/bridge/helm.tgz -C /usr/local/bin --strip-components=1 linux-amd64/helm
rm /opt/bridge/helm.tgz
fetch "https://github.com/kaimahi-agents/kaimahi/releases/download/$KMX_VERSION/kmx-linux-amd64" \
  "$KMX_LINUX_AMD64_SHA256" /usr/local/bin/kmx
chmod 0755 /usr/local/bin/kind /usr/local/bin/kubectl /usr/local/bin/helm /usr/local/bin/kmx

kmx_orka=$(kmx version | awk '$1 == "orka" { print $2 }')
if [ "$kmx_orka" != "$ORKA_VERSION" ]; then
  log "kmx $KMX_VERSION targets Orka $kmx_orka, not $ORKA_VERSION"
  exit 1
fi

log "kind cluster $CLUSTER"
install -d -m 0700 /etc/bridge-vm
docker pull -q "$KIND_NODE_IMAGE"
# The API server and the Orka API listen on the VM's only interface, which the host
# reaches through QEMU's user-mode network forwarded to its own loopback.
kind create cluster --name "$CLUSTER" --image "$KIND_NODE_IMAGE" --wait 5m --config - <<'EOF'
apiVersion: kind.x-k8s.io/v1alpha4
kind: Cluster
networking:
  apiServerAddress: 0.0.0.0
  apiServerPort: 6443
nodes:
- role: control-plane
  extraPortMappings:
  - containerPort: 30080
    hostPort: 8080
    listenAddress: 0.0.0.0
kubeadmConfigPatches:
- |
  kind: ClusterConfiguration
  apiServer:
    certSANs: ["127.0.0.1", "localhost"]
- |
  apiVersion: kubelet.config.k8s.io/v1beta1
  kind: KubeletConfiguration
  syncFrequency: 5s
EOF
kubectl config set-cluster "$CONTEXT" --server=https://127.0.0.1:6443 >/dev/null
docker update --restart=unless-stopped "$NODE" >/dev/null

log "Orka $ORKA_VERSION from its release chart"
fetch "https://github.com/orka-agents/orka/releases/download/$ORKA_VERSION/$ORKA_CHART_FILE" \
  "$ORKA_CHART_SHA256" /opt/bridge/orka-chart.tgz
mkdir /opt/bridge/chart
tar -xzf /opt/bridge/orka-chart.tgz -C /opt/bridge/chart
# Helm does not upgrade CRDs; apply the chart's own before installing, as kmx does.
kubectl apply -f /opt/bridge/chart/orka/crds/ >/dev/null
kubectl wait --for=condition=Established --timeout=120s -f /opt/bridge/chart/orka/crds/ >/dev/null
# The same release name, values and pinned digests kmx uses, so kmx adopts this install.
# IfNotPresent keeps boot offline-capable: every image is already in the node.
helm install orka /opt/bridge/orka-chart.tgz -n "$NAMESPACE" --create-namespace \
  --wait --timeout 15m \
  --set controller.mode=harness-v2 --set fullnameOverride=orka-api \
  --set-string 'labels.kmx\.kaimahi\.dev/installed=true' \
  --set-string "controller.image.digest=$ORKA_CONTROLLER_DIGEST" \
  --set-string "workers.ai.image.digest=$ORKA_AI_WORKER_DIGEST" \
  --set-string "workers.general.image.digest=$ORKA_GENERAL_WORKER_DIGEST" \
  --set-string "publisher.image.digest=$ORKA_PUBLISHER_DIGEST" \
  --set-string "controller.acpRuntime.codexImage=$ORKA_CODEX_IMAGE" \
  --set-string "controller.acpRuntime.claudeImage=$ORKA_CLAUDE_IMAGE" \
  --set-string "controller.acpRuntime.copilotImage=$ORKA_COPILOT_IMAGE" \
  --set-string "controller.acpRuntime.opencodeImage=$ORKA_OPENCODE_IMAGE" \
  --set controller.image.pullPolicy=IfNotPresent \
  --set publisher.image.pullPolicy=IfNotPresent \
  --set workers.ai.image.pullPolicy=IfNotPresent \
  --set workers.general.image.pullPolicy=IfNotPresent >/dev/null
rm -rf /opt/bridge/chart /opt/bridge/orka-chart.tgz

log "pre-pulling worker images"
for image in $ORKA_PREPULL_IMAGES; do
  log "  $image"
  docker exec "$NODE" crictl pull "$image" >/dev/null
done

log "Orka API on the VM's port 8080"
selector=$(kubectl -n "$NAMESPACE" get service orka-api -o jsonpath='{.spec.selector}')
kubectl apply -f - >/dev/null <<EOF
apiVersion: v1
kind: Service
metadata:
  name: orka-api-host
  namespace: $NAMESPACE
  labels:
    app.kubernetes.io/managed-by: orka-bridge-vm
spec:
  type: NodePort
  selector: $selector
  ports:
  - name: api
    port: 8080
    targetPort: api
    nodePort: 30080
EOF

log "checking Orka"
kubectl -n "$NAMESPACE" rollout status deployment/orka-api-controller --timeout=10m
for _ in $(seq 120); do
  if curl -fsS -o /dev/null http://127.0.0.1:8080/readyz; then break; fi
  sleep 1
done
curl -fsS -o /dev/null http://127.0.0.1:8080/readyz
# kmx must report this install as the release it pins, and its own install path must adopt
# the release unchanged, or the image is no use to it.
kmx cluster status --context "$CONTEXT" | tee /opt/bridge/kmx-status
grep -q "^version pinned by kmx  *$ORKA_VERSION$" /opt/bridge/kmx-status
grep -q "^version running  *${ORKA_VERSION#v} (release image digest)$" /opt/bridge/kmx-status
KAIMAHI_CONFIRM=$CONTEXT kmx cluster orka install --provider - --context "$CONTEXT" 2>&1 | tee /opt/bridge/kmx-status
grep -q "already installed by kmx; keeping its chart" /opt/bridge/kmx-status
[ "$(helm history orka -n "$NAMESPACE" | tail -n +2 | wc -l)" = 1 ]
rm /opt/bridge/kmx-status
docker exec "$NODE" crictl images
df -h /

log "boot service"
install -m 0755 /opt/bridge/bridge-vm-ready /usr/local/libexec/bridge-vm-ready
install -m 0644 /opt/bridge/bridge-vm-ready.service /etc/systemd/system/bridge-vm-ready.service
install -m 0644 /opt/bridge/image.env /etc/bridge-vm/image.env
systemctl enable bridge-vm-ready.service

ls -l /etc/netplan /etc/network/interfaces.d /etc/systemd/network 2>/dev/null || true
# Cloud-init wrote this boot's network by MAC address; replace it with plain DHCP on any
# virtio NIC so the image boots the same way on every host.
if [ -d /etc/netplan ]; then
  rm -f /etc/netplan/*.yaml
  cat >/etc/netplan/90-bridge-vm.yaml <<'EOF'
network:
  version: 2
  ethernets:
    nic:
      match:
        name: "en*"
      dhcp4: true
EOF
  chmod 0600 /etc/netplan/90-bridge-vm.yaml
fi

log "stopping the cluster for capture"
docker stop -t 60 "$NODE" >/dev/null

log "cleanup"
apt-get purge -y -q openssh-server unattended-upgrades >/dev/null 2>&1 || true
apt-get autoremove -y -q >/dev/null
apt-get clean
rm -rf /var/lib/apt/lists/* /var/cache/apt/*.bin
rm -f /opt/bridge/build.env /opt/bridge/image.env /opt/bridge/bridge-vm-ready*
cloud-init clean --logs --seed
touch /etc/cloud/cloud-init.disabled
journalctl --rotate >/dev/null 2>&1 || true
journalctl --vacuum-time=1s >/dev/null 2>&1 || true
truncate -s 0 /etc/machine-id
rm -f /var/lib/dbus/machine-id
fstrim -av
log "done"
