"""Synthetic model closure and bootstrap effects; never live-kind qualification."""
import copy
import json
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import jsonschema

from guest import lock as policy
from guest.cache import collect, inspect_archive
from guest.offline import member, stage_bundle
from guest.tests import test_lifecycle as lifecycle_tests
from guest.tests.fixtures import archive
from guest.tests.test_offline import SYNTHETIC_PROFILE, native_inputs

MODELS = ('vekil', 'model-launcher')


def model_inputs(root, architecture='amd64'):
    value, manifest = native_inputs(root, architecture)
    value['schemaVersion'] = 'bridge-guest-inputs/v2'
    value['components'] = [c for c in value['components'] if c['name'] != 'vekil']
    pins = {}
    for name in MODELS:
        component = archive(root, name, architecture, variant=name)
        value['components'].append(component)
        pins[name] = {'repository': component['repository'], architecture: component['manifestDigest']}
    return value, manifest, {SYNTHETIC_PROFILE['kmxSourceCommit']: pins}


class ModelImageTests(unittest.TestCase):
    def policy(self, pins):
        self.enterContext(patch.dict(policy.VERSIONS, {'sourceOfflineProfile': SYNTHETIC_PROFILE}))
        self.enterContext(patch.object(policy, 'MODEL_IMAGES', pins, create=True))

    def test_v2_models_stage_exact_refs_without_changing_core_manifest(self):
        for architecture in ('amd64', 'arm64'):
            with self.subTest(architecture=architecture), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                value, manifest, pins = model_inputs(root, architecture)
                self.policy(pins)
                original = (root / 'install.json').read_bytes()
                cache = root / 'cache'
                collect(value, cache, source=root)
                collected = policy.load_lock(cache / 'inputs.json')
                out = root / 'out'
                hashes = stage_bundle(collected, source=cache, out=out)
                self.assertEqual(original, (out / 'install.json').read_bytes())
                self.assertEqual(5, len(manifest['images']))
                staged = json.loads((out / 'inputs.json').read_bytes())
                for name in MODELS:
                    relative = 'components/' + name + '.oci.tar'
                    self.assertTrue((out / relative).is_file(), 'model closure must actually be staged: ' + name)
                    component = next(c for c in staged['components'] if c['name'] == name)
                    self.assertEqual(hashes[relative], component['archiveSha256'])
                    inspect_archive(component, out, architecture)
                    with tarfile.open(out / relative) as tar:
                        reference = json.load(member(tar, 'index.json'))['manifests'][0]['annotations']['org.opencontainers.image.ref.name']
                    self.assertEqual(component['repository'] + '@' + component['manifestDigest'], reference)
                roles = {c['name']: c for c in staged['components']}
                self.assertNotEqual(roles['session-loader']['manifestDigest'], roles['model-launcher']['manifestDigest'])
                self.assertFalse((out / 'components/telegram.oci.tar').exists())

    def test_v2_contract_and_v1_role_set_remain_distinct(self):
        schema = json.loads(Path(policy.__file__).with_name('input.schema.json').read_text())
        for architecture in ('amd64', 'arm64'):
            with self.subTest(architecture=architecture), tempfile.TemporaryDirectory() as temp:
                value, _, pins = model_inputs(Path(temp), architecture)
                self.policy(pins)
                policy.validate_lock(value, production=False)
                jsonschema.validate(value, schema)
                for version, remove_launcher in (('bridge-guest-inputs/v1', False), ('bridge-guest-inputs/v2', True)):
                    changed = copy.deepcopy(value)
                    changed['schemaVersion'] = version
                    if remove_launcher:
                        changed['components'] = [c for c in changed['components'] if c['name'] != 'model-launcher']
                    with self.assertRaises(ValueError):
                        policy.validate_lock(changed, production=False)
                    with self.assertRaises(jsonschema.ValidationError):
                        jsonschema.validate(changed, schema)
                legacy = copy.deepcopy(value)
                legacy['schemaVersion'] = 'bridge-guest-inputs/v1'
                legacy['components'] = [c for c in legacy['components'] if c['name'] != 'model-launcher']
                policy.validate_lock(legacy, production=False)
                jsonschema.validate(legacy, schema)

    def test_optional_original_model_index_is_preserved_and_verified(self):
        from guest.tests.fixtures import encoded, sha
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            value, _, pins = model_inputs(root)
            self.policy(pins)
            row = next(c for c in value['components'] if c['name'] == 'vekil')
            with tarfile.open(root / row['archive']) as tar:
                index = json.load(member(tar, 'index.json'))
            index['manifests'][0]['platform'] = {'os': 'linux', 'architecture': 'amd64'}
            raw = encoded(index)
            (root / 'model-index.json').write_bytes(raw)
            row.update(indexPath='model-index.json', indexDigest='sha256:' + sha(raw))
            out = root / 'out'
            stage_bundle(value, source=root, out=out)
            policy.load_lock(out / 'inputs.json')
            staged = json.loads((out / 'inputs.json').read_bytes())
            model = next(c for c in staged['components'] if c['name'] == 'vekil')
            self.assertEqual(raw, (out / model['indexPath']).read_bytes())
            inspect_archive(model, out, 'amd64')

    def test_wrong_model_pin_and_unknown_kmx_profile_do_not_stage(self):
        for mismatch in ('vekil', 'model-launcher', 'repository', 'profile'):
            with self.subTest(mismatch=mismatch), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                value, _, pins = model_inputs(root)
                self.policy(pins)
                if mismatch == 'profile':
                    self.enterContext(patch.object(policy, 'MODEL_IMAGES', {}))
                else:
                    row = next(c for c in value['components'] if c['name'] == ('vekil' if mismatch == 'repository' else mismatch))
                    if mismatch == 'repository':
                        row['repository'] = 'registry.example.com/other'
                    else:
                        row['manifestDigest'] = 'sha256:' + 'b' * 64
                with self.assertRaisesRegex(ValueError, 'model image'):
                    stage_bundle(value, source=root, out=root / 'out')
                self.assertFalse((root / 'out').exists())

    def harness(self, architecture='amd64', wrong_cri=False):
        harness = lifecycle_tests.LifecycleTests('runTest')
        harness.setUp()
        self.addCleanup(harness.doCleanups)
        source = harness.root / 'models'
        source.mkdir()
        value, manifest, pins = model_inputs(source, architecture)
        self.policy(pins)
        harness.bundle = harness.root / 'model-bundle'
        stage_bundle(value, source=source, out=harness.bundle)
        harness.lock = json.loads((harness.bundle / 'inputs.json').read_bytes())
        harness.manifest = manifest
        self.enterContext(patch.object(harness.api, 'BUNDLE', harness.bundle))
        original = harness.execute
        imports = []
        def execute(argv, *, env=None, stdin=None, timeout=None):
            if 'import' in argv:
                imports.append(stdin)
            if 'inspecti' in argv:
                harness.calls.append((argv, env))
                reference = argv[-1]
                component = next(c for c in harness.lock['components'] if reference == c['repository'] + '@' + c['manifestDigest'])
                digest = 'sha256:' + 'f' * 64 if wrong_cri and component['name'] == 'model-launcher' else component['configDigest']
                return json.dumps({'status': {'id': digest, 'repoDigests': [reference]}})
            return original(argv, env=env, stdin=stdin, timeout=timeout)
        self.enterContext(patch.object(harness.api, 'execute', execute))
        return harness, imports

    def check_model_handoff(self, architecture):
        harness, imports = self.harness(architecture)
        result = harness.api.bootstrap(harness.lock, harness.state)
        self.assertEqual('infrastructure-ready', result['phase'])
        for name in MODELS:
            self.assertIn(harness.bundle / ('components/' + name + '.oci.tar'), imports)
        up = next(args for args, _ in harness.calls if args[0] == 'kmx' and 'up' in args)
        self.assertEqual('copilot-v1', up[up.index('--credential-mount') + 1])
        self.assertFalse(any('connect' in args for args, _ in harness.calls))
        harness.calls.clear()
        harness.api.resume(harness.lock, harness.state)
        up = next(args for args, _ in harness.calls if args[0] == 'kmx' and 'up' in args)
        self.assertEqual('copilot-v1', up[up.index('--credential-mount') + 1])

    # Separate cases keep the existing lifecycle fixture's stopall cleanup nested.
    def test_models_use_existing_import_cri_checks_and_first_cluster_mount(self):
        self.check_model_handoff('amd64')

    def test_arm_models_use_existing_import_cri_checks_and_first_cluster_mount(self):
        self.check_model_handoff('arm64')

    def test_first_cluster_creation_selects_delivered_credential_mount(self):
        harness, _ = self.harness()
        harness.api.bootstrap(harness.lock, harness.state)
        up = next(args for args, _ in harness.calls if args[0] == 'kmx' and 'up' in args)
        self.assertIn('--credential-mount', up)
        self.assertEqual('copilot-v1', up[up.index('--credential-mount') + 1])

    def test_wrong_model_cri_identity_cannot_mark_cache_or_install_orka(self):
        harness, _ = self.harness(wrong_cri=True)
        with self.assertRaisesRegex(ValueError, 'exact CRI image unavailable'):
            harness.api.bootstrap(harness.lock, harness.state)
        self.assertEqual('cluster-ready', json.loads((harness.state / 'bootstrap.json').read_bytes())['phase'])
        self.assertFalse(any('install' in args for args, _ in harness.calls))

    def test_tampered_model_archive_is_checked_before_any_bootstrap_effect(self):
        harness, _ = self.harness()
        (harness.bundle / 'components/vekil.oci.tar').write_bytes(b'tampered')
        with self.assertRaises(ValueError):
            harness.api.bootstrap(harness.lock, harness.state)
        self.assertEqual([], harness.calls)
        self.assertFalse((harness.state / 'bootstrap.json').exists())
