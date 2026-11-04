import importlib
import json
import socket
import threading
import unittest
import uuid


class ExchangeTests(unittest.TestCase):
    def test_real_wire_early_then_existing_serve_and_readiness(self):
        try:
            api = importlib.import_module("guest.reference_host")
        except ModuleNotFoundError:
            self.fail("native reference host exchange missing")
        boot = {"installationId": str(uuid.uuid4()), "generation": 1, "bootNonce": str(uuid.uuid4()), "stateCreation": "new", "stateFsUuid": "id", "stateBytes": 34359738368, "hostUtc": 1}
        host, guest = socket.socketpair()
        self.addCleanup(host.close)
        self.addCleanup(guest.close)
        received, errors = [], []
        def exchange():
            try:
                received.extend(api.runtime_exchange(host, boot))
            except (ValueError, OSError, KeyError) as error:
                errors.append(error)
        thread = threading.Thread(target=exchange, daemon=True)
        thread.start()
        io = guest.makefile("rwb", buffering=0)
        for stage, id_ in (("early", 1), ("serve", 2)):
            hello = {"type": "hello", "body": {"version": 1, "stage": stage, "guestNonce": str(uuid.uuid4()), "acceptedBootNonce": None if stage == "early" else boot["bootNonce"]}}
            io.write(json.dumps(hello).encode() + b"\n")
            request = json.loads(io.readline())
            self.assertEqual(id_, request["body"]["requestId"])
            self.assertEqual("new" if stage == "early" else "existing", request["body"]["body"]["body"]["stateCreation"])
            result = {"type": "result", "body": {"version": 1, "bootNonce": boot["bootNonce"], "requestId": id_, "body": {"op": "ready", "body": {}}}}
            io.write(json.dumps(result).encode() + b"\n")
        request = json.loads(io.readline())
        self.assertEqual("readiness", request["body"]["body"]["op"])
        result["body"]["requestId"] = 3
        io.write(json.dumps(result).encode() + b"\n")
        thread.join(2)
        self.assertFalse(thread.is_alive())
        self.assertEqual([], errors)
        self.assertEqual(5, len(received))
