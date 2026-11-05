"""JSON Schema parity tests; the validator is a development-only dependency."""
import copy
import importlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

from images import package
from images.test_package import BoundedStream, boot_inputs, metadata


@unittest.skipUnless(importlib.util.find_spec("jsonschema"), "Install python3-jsonschema to run schema tests")
class SchemaTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        library = importlib.import_module("jsonschema")
        schema = json.loads(Path(package.__file__).with_name("manifest.schema.json").read_text())
        library.Draft202012Validator.check_schema(schema)
        self.validator = library.Draft202012Validator(schema, format_checker=library.FormatChecker())

    def manifest(self, platform):
        root = self.root / platform
        root.mkdir()
        meta = metadata(root, platform)
        payload = b"synthetic schema fixture, not a bootable image"
        return package.create(BoundedStream(payload), meta, root / "package",
                              **boot_inputs(root / "boot", meta, payload))

    def test_complete_role_sets_validate_for_both_targets(self):
        for platform in ("windows", "macos"):
            with self.subTest(platform=platform):
                manifest = self.manifest(platform)
                self.validator.validate(manifest)
                package.validate_manifest(manifest)

    def test_duplicate_driver_name_cannot_replace_required_driver(self):
        for platform in ("windows", "macos"):
            with self.subTest(platform=platform):
                manifest = self.manifest(platform)
                self.validator.validate(manifest)
                bad = copy.deepcopy(manifest)
                rows = bad["directBoot"]["requiredDrivers"]
                rows[-1] = dict(rows[0], builtIn=not rows[0]["builtIn"])
                self.assertEqual(len(rows), len({json.dumps(row, sort_keys=True) for row in rows}))
                self.assertEqual({"contains"}, {error.validator for error in self.validator.iter_errors(bad)})
                with self.assertRaisesRegex(ValueError, "required drivers"):
                    package.validate_manifest(bad)

    def test_duplicate_tool_name_cannot_replace_required_extractor(self):
        for platform in ("windows", "macos"):
            with self.subTest(platform=platform):
                manifest = self.manifest(platform)
                self.validator.validate(manifest)
                bad = copy.deepcopy(manifest)
                rows = bad["directBoot"]["provenance"]["extractionTools"]
                rows[-1] = dict(rows[0], version="different valid version")
                self.assertEqual(len(rows), len({json.dumps(row, sort_keys=True) for row in rows}))
                self.assertEqual({"contains"}, {error.validator for error in self.validator.iter_errors(bad)})
                with self.assertRaisesRegex(ValueError, "extraction tool"):
                    package.validate_manifest(bad)
