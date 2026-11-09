"""Loopback-only discovery fixture for the CI native card walk; no cluster or credentials."""
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import sys


class Discovery(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        path = self.path.split("?")[0]
        if path.endswith("/services"):
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


if __name__ == "__main__":
    server = ThreadingHTTPServer(("127.0.0.1", 0), Discovery)
    # The caller supplies its CI scratch path; the bearer below is a synthetic fixture.
    Path(sys.argv[1]).write_text(json.dumps({
        "apiVersion": "v1", "kind": "Config", "current-context": "fixture-cluster",
        "clusters": [{"name": "fixture", "cluster": {"server": f"http://127.0.0.1:{server.server_port}"}}],
        "contexts": [{"name": "fixture-cluster", "context": {"cluster": "fixture", "user": "fixture"}}],
        "users": [{"name": "fixture", "user": {"token": "ci-fixture-not-a-credential"}}],
    }))
    server.serve_forever()
