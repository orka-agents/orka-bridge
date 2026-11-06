"""Persistent byte-exact consumer vectors; fixtures confer no distribution approval."""
import gzip
import hashlib
import json
import tempfile
import unittest
from pathlib import Path

import jsonschema
from images import package
from images.test_package import BoundedStream, boot_inputs, metadata

ROOT = Path(__file__).with_name("testdata") / "v2"


class ConsumerVectors(unittest.TestCase):
    def test_shared_structure_verdicts(self):
        schema = json.loads(Path(package.__file__).with_name("manifest.schema.json").read_bytes())
        validator = jsonschema.Draft202012Validator(schema)
        for row in json.loads((ROOT / "vectors.json").read_bytes()):
            with self.subTest(invariant=row["invariant"]):
                raw = (ROOT / row["file"]).read_bytes()
                self.assertEqual(hashlib.sha256(raw).hexdigest(), row["sha256"])
                try:
                    value = package.guest.read_json(raw)
                    validator.validate(value)
                    package.validate_manifest(value)
                except (ValueError, jsonschema.ValidationError):
                    verdict = "invalid"
                else:
                    verdict = "valid"
                self.assertEqual(verdict, row["structure"])
                # Version ordering is consumer compatibility, not structural corruption.
                if row["compatibility"] == "UnsupportedVersionPolicy":
                    self.assertEqual(verdict, "valid")

    def test_positive_bytes_come_from_actual_producer_helpers(self):
        with tempfile.TemporaryDirectory() as temporary:
            for platform in ("macos", "windows"):
                root = Path(temporary) / platform
                root.mkdir()
                meta = metadata(root, platform)
                payload = b"synthetic consumer fixture, never bootable\n"
                value = package.create(BoundedStream(payload), meta, root / "package", 17,
                                       **boot_inputs(root / "boot", meta, payload))
                self.assertEqual(package.canonical(value), (ROOT / ("producer-" + platform + ".json")).read_bytes())

    def test_future_gzip_data_documents_python_behavior_only(self):
        for row in json.loads((ROOT / "gzip-future.json").read_bytes()):
            with self.subTest(invariant=row["invariant"]):
                raw = bytes.fromhex(row["hex"])
                if row["valid"]:
                    self.assertEqual(gzip.decompress(raw).decode(), row["payload"])
                else:
                    with self.assertRaises((OSError, EOFError)):
                        gzip.decompress(raw)
