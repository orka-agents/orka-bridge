"""Exercise the CI-only native Send fixture over the real port-forward wire."""
import importlib.util
import json
from pathlib import Path
import socket
import struct
import threading
import unittest
from http.server import ThreadingHTTPServer

spec = importlib.util.spec_from_file_location("fixture", Path(__file__).with_name("cluster-card-fixture.py"))
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)


class SendFixtureTest(unittest.TestCase):
    def test_conditional_and_denied_access_are_distinct(self):
        handler = object.__new__(fixture.Discovery)
        status, identity = handler.orka("GET", "/api/v1/auth/whoami", None, "Bearer conditional-fixture")
        self.assertEqual(status, 200)
        self.assertEqual(identity["authType"], "tokenReview")
        self.assertEqual(identity["namespace"], "")
        status, _ = handler.orka("GET", "/api/v1/auth/whoami", None, "Bearer denied-fixture")
        self.assertEqual(status, 403)

    def test_native_forward_carries_create_receipts_and_check_evidence(self):
        server = ThreadingHTTPServer(("127.0.0.1", 0), fixture.Discovery)
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        with socket.create_connection(server.server_address, timeout=2) as sock:
            sock.sendall(b"GET /api/v1/namespaces/control/pods/ready/portforward?ports=8080 HTTP/1.1\r\nHost: fixture\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: v4.channel.k8s.io\r\n\r\n")
            wire = sock.makefile("rb")
            self.assertIn(b"101 Switching Protocols", wire.readline())
            while wire.readline() != b"\r\n":
                pass

            def frame():
                opcode, length = wire.read(2)
                self.assertEqual(opcode, 130)
                if length == 126:
                    length = struct.unpack("!H", wire.read(2))[0]
                elif length == 127:
                    length = struct.unpack("!Q", wire.read(8))[0]
                return wire.read(length)

            self.assertEqual(frame(), b"\x00\x90\x1f")
            self.assertEqual(frame(), b"\x01\x90\x1f")

            def request(path, body=None):
                data = json.dumps(body).encode() if body is not None else b""
                method = "POST" if body is not None else "GET"
                http = f"{method} /api/v1/{path}?namespace=agents HTTP/1.1\r\nHost: fixture\r\nAuthorization: Bearer ci-fixture-not-a-credential\r\nContent-Length: {len(data)}\r\n\r\n".encode() + data
                payload = b"\x00" + http
                mask = b"1234"
                size = bytes([len(payload)]) if len(payload) < 126 else b"\x7e" + struct.pack("!H", len(payload))
                sock.sendall(b"\x82" + bytes([size[0] | 128]) + size[1:] + mask + bytes(v ^ mask[i % 4] for i, v in enumerate(payload)))
                reply = frame()
                return json.loads(reply.split(b"\r\n\r\n", 1)[1])

            self.assertEqual(request("auth/whoami")["namespace"], "agents")
            created = request("agents", {"name": "probe", "namespace": "agents", "spec": {"model": {"name": "exact"}}})
            self.assertEqual(created["metadata"]["name"], "probe")
            self.assertEqual(created["spec"]["model"]["name"], "exact")
            task = request("tasks", {"name": "check", "namespace": "agents", "spec": {"agentRef": {"name": "probe"}, "timeout": "120s", "retryPolicy": {"maxRetries": 0}}})
            self.assertEqual(task["status"]["phase"], "Succeeded")
            self.assertEqual(request("tasks/check")["metadata"]["uid"], task["metadata"]["uid"])
            self.assertIn("cf699c", request("tasks/check/result")["result"])
            self.assertEqual(request("tasks/check/events")["events"][0]["toolName"], "web_fetch")
            request("agents", {"name": "probe2", "namespace": "agents", "spec": {}})
            second = request("tasks", {"name": "check2", "namespace": "agents", "spec": {"agentRef": {"name": "probe2"}}})
            self.assertEqual(second["status"]["phase"], "Running")
            wire.close()


if __name__ == "__main__":
    unittest.main()
