"""Loopback-only native Send walk fixture; no real cluster, model or credentials."""
import base64
import hashlib
import json
import struct
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import sys


class Discovery(BaseHTTPRequestHandler):
    objects = {}
    agents = 0
    lock = threading.Lock()

    def log_message(self, *_):
        pass

    def do_GET(self):
        path = self.path.split("?")[0]
        if path.endswith("/portforward"):
            self.forward()
            return
        if path.endswith("/pods"):
            value = {"apiVersion": "v1", "kind": "PodList", "items": [{
                "metadata": {"name": "ready", "namespace": "control", "uid": "pod-fixture", "labels": {"app": "orka"}},
                "spec": {"containers": [{"name": "controller", "ports": [{"containerPort": 8080}]}]},
                "status": {"phase": "Running", "conditions": [{"type": "Ready", "status": "True"}]},
            }]}
        elif path.endswith("/services"):
            value = {"apiVersion": "v1", "kind": "ServiceList", "items": [{
                "metadata": {"name": "orka", "namespace": "control", "uid": "service-fixture"},
                "spec": {"selector": {"app": "orka"}, "ports": [{"name": "api", "port": 80, "targetPort": 8080}]},
            }]}
        elif path.endswith("/deployments"):
            value = {"apiVersion": "apps/v1", "kind": "DeploymentList", "items": [{
                "metadata": {"name": "controller", "namespace": "control", "uid": "deployment-fixture"},
                "spec": {"selector": {"matchLabels": {"app": "orka"}}, "template": {
                    "metadata": {"labels": {"app": "orka"}}, "spec": {"containers": [{
                        "name": "controller", "image": "ghcr.io/orka-agents/orka:fixture",
                        "args": ["--api-port=8080", "--watch-namespace=agents"],
                        "ports": [{"containerPort": 8080}],
                    }]},
                }},
            }]}
        elif "/customresourcedefinitions/" in path:
            plural = path.rsplit("/", 1)[1].split(".")[0]
            value = {"apiVersion": "apiextensions.k8s.io/v1", "kind": "CustomResourceDefinition",
                     "metadata": {"name": f"{plural}.core.orka.ai"},
                     "spec": {"group": "core.orka.ai", "scope": "Namespaced",
                              "names": {"kind": "Fixture", "plural": plural},
                              "versions": [{"name": "v1alpha1", "served": True, "storage": True}]}}
        else:
            self.send_error(404)
            return
        body = json.dumps(value).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def frame(self, payload):
        length = len(payload)
        size = bytes([length]) if length < 126 else b"\x7e" + struct.pack("!H", length) if length < 65536 else b"\x7f" + struct.pack("!Q", length)
        self.wfile.write(b"\x82" + size + payload)
        self.wfile.flush()

    def forward(self):
        key = self.headers["Sec-WebSocket-Key"]
        accept = base64.b64encode(hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()).decode()
        self.protocol_version = "HTTP/1.1"
        self.send_response(101, "Switching Protocols")
        for name, value in [("Connection", "Upgrade"), ("Upgrade", "websocket"), ("Sec-WebSocket-Accept", accept), ("Sec-WebSocket-Protocol", "v4.channel.k8s.io")]:
            self.send_header(name, value)
        self.end_headers()
        self.close_connection = True
        try:
            self.frame(b"\x00\x90\x1f")
            self.frame(b"\x01\x90\x1f")
            pending = b""
            while header := self.rfile.read(2):
                opcode, size = header
                if opcode & 15 == 8:
                    return
                length = size & 127
                if length == 126:
                    length = struct.unpack("!H", self.rfile.read(2))[0]
                elif length == 127:
                    length = struct.unpack("!Q", self.rfile.read(8))[0]
                if length > 1024 * 1024 or not size & 128:
                    return
                mask = self.rfile.read(4)
                data = self.rfile.read(length)
                data = bytes(v ^ mask[i % 4] for i, v in enumerate(data))
                if opcode & 15 != 2 or not data or data[0] != 0:
                    return
                pending += data[1:]
                while b"\r\n\r\n" in pending:
                    head, body = pending.split(b"\r\n\r\n", 1)
                    lines = head.decode().split("\r\n")
                    headers = dict(line.lower().split(":", 1) for line in lines[1:] if ":" in line)
                    length = int(headers.get("content-length", "0"))
                    if len(body) < length:
                        break
                    method, route, _ = lines[0].split(" ")
                    status, value = self.orka(method, route.split("?")[0], json.loads(body[:length]) if length else None, headers.get("authorization", "").strip())
                    encoded = json.dumps(value).encode()
                    reply = f"HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {len(encoded)}\r\n\r\n".encode() + encoded
                    self.frame(b"\x00" + reply)
                    pending = body[length:]
        except (BrokenPipeError, ConnectionResetError, OSError):
            pass

    def orka(self, method, route, body, authorization=""):
        if authorization == "Bearer denied-fixture":
            return 403, {}
        if route.endswith("/auth/whoami") and authorization == "Bearer conditional-fixture":
            return 200, {"authenticated": True, "authType": "tokenReview", "namespace": ""}
        with self.lock:
            if method == "POST":
                kind = "Agent" if route.endswith("/agents") else "Task"
                if kind == "Agent":
                    type(self).agents += 1
                value = {"apiVersion": "core.orka.ai/v1alpha1", "kind": kind,
                         "metadata": {"name": body["name"], "namespace": "agents", "uid": "uid-" + body["name"], "generation": 1},
                         "spec": body["spec"], "status": {"phase": "Succeeded" if self.agents % 2 else "Running"}}
                if kind == "Task" and "timeout" in value["spec"]:
                    value["spec"]["timeout"] = "2m0s"
                    value["spec"]["retryPolicy"].pop("maxRetries", None)
                self.objects[route + "/" + body["name"]] = value
                return 201, value
            if route.endswith("/auth/whoami"):
                value = {"authenticated": True, "authType": "oidc", "namespace": "agents"}
            elif route.endswith("/providers"):
                value = {"items": [{"metadata": {"name": "fixture-model", "namespace": "agents", "uid": "provider-fixture"}, "status": {"ready": True}}], "metadata": {}}
            elif route.endswith("/tools"):
                value = {"items": [{"name": "web_fetch", "builtin": True}], "metadata": {}}
            elif route.endswith("/agents"):
                value = {"items": [], "metadata": {}}
            elif route.endswith("/result"):
                value = {"result": "Fixture answer: cf699c56340dc775230fde4ef84237d27563ea6ef52164c7d078072b586c3918"}
            elif route.endswith("/events"):
                task = route.rsplit("/", 2)[1]
                agent = self.objects[route.rsplit("/", 1)[0]]["spec"]["agentRef"]["name"]
                event = {"namespace": "agents", "streamType": "task", "streamID": task, "seq": 1, "type": "ToolCallCompleted", "taskName": task, "agentName": agent, "toolName": "web_fetch", "toolCallID": "fixture-call"}
                value = {"namespace": "agents", "streamType": "task", "streamID": task, "afterSeq": 0, "latestSeq": 1, "events": [event]}
            elif route in self.objects:
                value = self.objects[route]
            else:
                return 404, {}
            return 200, value


if __name__ == "__main__":
    server = ThreadingHTTPServer(("127.0.0.1", 0), Discovery)
    # The caller supplies its CI scratch path; the bearer below is a synthetic fixture.
    Path(sys.argv[1]).write_text(json.dumps({
        "apiVersion": "v1", "kind": "Config", "current-context": "fixture-cluster",
        "clusters": [{"name": "fixture", "cluster": {"server": f"http://127.0.0.1:{server.server_port}"}}],
        "contexts": [{"name": name, "context": {"cluster": "fixture", "user": user}} for name, user in [("fixture-cluster", "fixture"), ("conditional-fixture", "conditional"), ("denied-fixture", "denied")]],
        "users": [{"name": name, "user": {"token": token}} for name, token in [("fixture", "ci-fixture-not-a-credential"), ("conditional", "conditional-fixture"), ("denied", "denied-fixture")]],
    }))
    server.serve_forever()
