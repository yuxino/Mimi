#!/usr/bin/env python3
"""Offline scheduler contracts; never starts credentials or provider work."""
import importlib.util
import json
from pathlib import Path
import tempfile
import threading
import time
import unittest
import wave

spec = importlib.util.spec_from_file_location('batch', Path(__file__).with_name('run-development-batch.py'))
batch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(batch)


class BatchTests(unittest.TestCase):
    def fixture(self, root, label='scene', expected=True):
        pcm = root / (label + '.pcm')
        pcm.write_bytes(b'\x00\x01' * 1600)
        wav = root / (label + '.wav')
        with wave.open(str(wav), 'wb') as output:
            output.setparams((1, 2, 16000, 0, 'NONE', 'not compressed'))
            output.writeframes(pcm.read_bytes())
        reference = root / (label + '.txt')
        reference.write_text('synthetic speech' if expected else '')
        return {'label': label, 'pcmPath': str(pcm), 'wavPath': str(wav),
                'referencePath': str(reference) if expected else None,
                'asrLanguage': 'auto', 'unit': 'word', 'expectedSpeech': expected,
                'tags': ['test'], 'provenance': {'license': 'CC0-1.0',
                'sourceUrl': 'https://example.org/public-source', 'referenceStatus': 'synthetic-test'}}

    def test_preparation_proves_same_pcm_and_preserves_source(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            scene = self.fixture(root)
            prepared = batch.prepare_scene(scene)
            original = (root / 'scene.pcm').read_bytes()
            staged = batch.stage_scene(prepared, root / 'control')
            child = json.loads(staged['manifestPath'].read_text())['clips'][0]
            self.assertEqual(Path(child['pcm_path']).read_bytes(), original)
            self.assertEqual((root / 'scene.pcm').read_bytes(), original)
            self.assertTrue(Path(child['pcm_path']).is_relative_to(staged['manifestPath'].parent))
            self.assertEqual(staged['pcmSha256'], prepared['pcmSha256'])

    def test_mismatched_audition_audio_fails_before_any_executor(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            scene = self.fixture(root)
            Path(scene['pcmPath']).write_bytes(b'\x00\x02' * 1600)
            with self.assertRaisesRegex(batch.BatchError, 'pcm_wav_mismatch'):
                batch.prepare_scene(scene)

    def test_negative_control_has_no_empty_denominator_reference(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            scene = self.fixture(root, expected=False)
            prepared = batch.prepare_scene(scene)
            staged = batch.stage_scene(prepared, root / 'negative')
            child = json.loads(staged['manifestPath'].read_text())['clips'][0]
            self.assertIsNone(child['reference_path'])
            self.assertIsNone(prepared['referenceBytes'])
            scene['referencePath'] = str(root / 'scene.txt')
            with self.assertRaises(batch.BatchError):
                batch.prepare_scene(scene)

    def test_a_reference_cannot_redirect_to_a_credential_file(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            scene = self.fixture(root)
            scene['referencePath'] = str(root / '.env')
            with self.assertRaisesRegex(batch.BatchError, 'speech_reference_required'):
                batch.prepare_scene(scene)

    def test_duplicate_labels_and_non_public_provenance_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            scene = self.fixture(root)
            with self.assertRaisesRegex(batch.BatchError, 'duplicate_label'):
                batch.validate_matrix({'schemaVersion': 1, 'scenes': [scene, scene]})
            scene['provenance']['sourceUrl'] = 'file:///private/location'
            with self.assertRaises(batch.BatchError):
                batch.prepare_scene(scene)

    def test_symlinks_oversize_and_wrong_wav_format_do_not_get_uploaded(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            scene = self.fixture(root)
            original = root / 'source.pcm'
            Path(scene['pcmPath']).rename(original)
            try:
                Path(scene['pcmPath']).symlink_to(original)
            except OSError:
                return
            with self.assertRaises(batch.BatchError):
                batch.prepare_scene(scene)
            Path(scene['pcmPath']).unlink()
            Path(scene['pcmPath']).write_bytes(b'\0' * (batch.MAX_PCM_BYTES + 2))
            with self.assertRaises(batch.BatchError):
                batch.prepare_scene(scene)
            Path(scene['pcmPath']).write_bytes(original.read_bytes())
            with wave.open(scene['wavPath'], 'wb') as output:
                output.setparams((1, 2, 8000, 0, 'NONE', 'not compressed'))
                output.writeframes(original.read_bytes())
            with self.assertRaises(batch.BatchError):
                batch.prepare_scene(scene)

    def test_concurrency_bound_failure_isolation_and_incremental_progress(self):
        lock = threading.Lock()
        active = maximum = 0
        progress = []

        def execute(scene, cancelled):
            nonlocal active, maximum
            with lock:
                active += 1
                maximum = max(maximum, active)
            time.sleep(0.02)
            with lock:
                active -= 1
            if scene['label'] == '2':
                raise batch.BatchError('provider_job_failed')
            return {'label': scene['label'], 'status': 'complete'}

        rows = batch.run_jobs([{'label': str(i)} for i in range(8)], 3, execute,
                              lambda rows: progress.append([dict(row) for row in rows]))
        self.assertEqual(maximum, 3)
        self.assertEqual(sum(row['status'] == 'complete' for row in rows), 7)
        self.assertEqual(rows[2]['failure'], 'provider_job_failed')
        self.assertTrue(any(row['status'] == 'running' for page in progress for row in page))
        self.assertEqual([row['label'] for row in rows], [str(i) for i in range(8)])

    def test_cancellation_stops_queued_jobs_and_signals_running_workers(self):
        cancelled = threading.Event()
        called = []

        def execute(scene, signal):
            called.append(scene['label'])
            signal.set()
            return {'label': scene['label'], 'status': 'cancelled'}

        rows = batch.run_jobs([{'label': str(i)} for i in range(12)], 1, execute,
                              lambda rows: None, cancelled)
        self.assertEqual(called, ['0'])
        self.assertTrue(all(row['status'] == 'cancelled' for row in rows))

    def test_provider_command_is_exact_and_child_environment_has_no_secret_fallback(self):
        staged = {'manifestPath': Path('/private/control/manifest.json'),
                  'outputDirectory': Path('/private/tmp/mimi-debug-benchmark/job')}
        args, environment = batch.child_command(Path('/test/executable'), staged,
                                                {'PATH': '/bin', 'MIMI_ASR_BENCH_ARM': 'model31', 'API_KEY': 'never-inherit'})
        self.assertEqual(args, ['/test/executable', batch.TEST_NAME, '--ignored', '--exact', '--nocapture'])
        self.assertEqual(environment['MIMI_ASR_BENCH_ARM'], 'baseline')
        self.assertEqual(environment['MIMI_ASR_BENCH_DENOISE'], 'bypass')
        self.assertEqual(environment['MIMI_ASR_BENCH_MANIFEST'], str(staged['manifestPath']))
        self.assertFalse(any('KEY' in key or 'TOKEN' in key for key in environment))


if __name__ == '__main__':
    unittest.main()
