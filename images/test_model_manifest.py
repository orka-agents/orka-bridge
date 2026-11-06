"""Producer and consumer parity for the explicit v2 guest input contract."""
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import jsonschema

from images import package
from images.guest.tests.fixtures import archive
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
    commit = package.guest.VERSIONS['sourceOfflineProfile']['kmxSourceCommit']
    payload = b'synthetic consumer fixture, never bootable\n'
    with patch.object(package.guest, 'MODEL_IMAGES', {commit: pins}):
        return package.create(BoundedStream(payload), meta, root / 'package', 17,
                              **boot_inputs(root / 'boot', meta, payload))


class ModelManifestTests(unittest.TestCase):
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
