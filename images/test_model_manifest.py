"""Producer and consumer parity for the explicit v2 guest input contract."""
import copy
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import jsonschema

from images import package
from images.guest.tests.fixtures import archive, lock
from images.test_package import BoundedStream, boot_inputs, metadata


def model_manifest(root, platform='windows'):
    meta = metadata(root, platform)
    lock = meta['componentLock']
    lock['schemaVersion'] = 'bridge-guest-inputs/v2'
    lock['components'] = [c for c in lock['components'] if c['name'] != 'vekil']
    pins = {}
    for name in ('vekil', 'model-launcher'):
        component = archive(root, name, meta['architecture'], variant=name)
        lock['components'].append(component)
        pins[name] = {'repository': component['repository'], meta['architecture']: component['manifestDigest']}
    schema = json.loads(Path(package.guest.__file__).with_name('input.schema.json').read_text())
    jsonschema.validate(lock, schema)
    commit = package.guest.VERSIONS['sourceOfflineProfile']['kmxSourceCommit']
    payload = b'synthetic consumer fixture, never bootable\n'
    with patch.object(package.guest, 'MODEL_IMAGES', {commit: pins}):
        return package.create(BoundedStream(payload), meta, root / 'package', 17,
                              **boot_inputs(root / 'boot', meta, payload))


class ModelManifestTests(unittest.TestCase):
    def test_guest_input_schema_keeps_versioned_role_closures(self):
        schema = json.loads(Path(package.guest.__file__).with_name('input.schema.json').read_text())
        for architecture in ('amd64', 'arm64'):
            with self.subTest(architecture=architecture), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                value = lock(root, architecture)
                jsonschema.validate(value, schema)
                value['schemaVersion'] = 'bridge-guest-inputs/v2'
                value['components'].append(archive(root, 'model-launcher', architecture))
                jsonschema.validate(value, schema)
                for version, remove_launcher in (('bridge-guest-inputs/v1', False), ('bridge-guest-inputs/v2', True)):
                    changed = copy.deepcopy(value)
                    changed['schemaVersion'] = version
                    if remove_launcher:
                        changed['components'] = [c for c in changed['components'] if c['name'] != 'model-launcher']
                    with self.assertRaises(jsonschema.ValidationError):
                        jsonschema.validate(changed, schema)

    def test_real_producer_accepts_v2_input_and_shared_schema(self):
        schema = json.loads(Path(package.__file__).with_name('manifest.schema.json').read_text())
        for platform in ('windows', 'macos'):
            with self.subTest(platform=platform), tempfile.TemporaryDirectory() as temp:
                value = model_manifest(Path(temp), platform)
                self.assertEqual('bridge-guest-inputs/v2', value['components']['inputSchemaVersion'])
                jsonschema.validate(value, schema)
                package.validate_manifest(value)
                if platform == 'windows':
                    golden = Path(package.__file__).parent / 'testdata/v2/producer-model-input-v2.json'
                    self.assertEqual(package.canonical(value), golden.read_bytes())
