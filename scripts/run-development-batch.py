#!/usr/bin/env python3
"""Opted-in, bounded manual ASR batches. No application or credential fallback."""
import argparse
from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import threading
import time
import uuid
import unicodedata
import wave

TEST_NAME = 'audio3_benchmark::manual_same_pcm_asr_comparison'
MAX_PCM_BYTES = 60 * 32000
MAX_REFERENCE_BYTES = 64 * 1024
MAX_MATRIX_BYTES = 1024 * 1024
MAX_STAGED_BYTES = 64 * 1024 * 1024
MAX_SCENES = 32
OUTPUT_PARENT = Path('/private/tmp/mimi-debug-benchmark')
LABEL = re.compile(r'[A-Za-z0-9][A-Za-z0-9_-]{0,47}\Z')
LICENSES = {'CC-BY-4.0', 'CC-BY-3.0', 'CC-BY-2.5', 'CC-BY-SA-4.0', 'CC-BY-SA-3.0', 'CC0-1.0', 'PublicDomain'}


class BatchError(Exception):
    pass


def bounded_regular(path, maximum):
    path = Path(path)
    if not path.is_absolute():
        raise BatchError('input_path_invalid')
    try:
        before = path.lstat()
        if not stat.S_ISREG(before.st_mode) or before.st_size > maximum:
            raise BatchError('input_file_invalid')
        fd = os.open(path, os.O_RDONLY | getattr(os, 'O_NOFOLLOW', 0) | getattr(os, 'O_NONBLOCK', 0))
        with os.fdopen(fd, 'rb') as source:
            after = os.fstat(source.fileno())
            if not stat.S_ISREG(after.st_mode) or (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino):
                raise BatchError('input_file_changed')
            data = source.read(maximum + 1)
        if len(data) > maximum:
            raise BatchError('input_file_invalid')
        return data
    except OSError:
        raise BatchError('input_read_failed') from None


def validate_matrix(matrix):
    if not isinstance(matrix, dict) or matrix.get('schemaVersion') != 1:
        raise BatchError('batch_manifest_invalid')
    scenes = matrix.get('scenes')
    if not isinstance(scenes, list) or not 1 <= len(scenes) <= MAX_SCENES:
        raise BatchError('scene_count_invalid')
    labels = set()
    for scene in scenes:
        label = scene.get('label') if isinstance(scene, dict) else None
        if not isinstance(label, str) or not LABEL.fullmatch(label):
            raise BatchError('scene_label_invalid')
        if label in labels:
            raise BatchError('duplicate_label')
        labels.add(label)
    return scenes


def prepare_scene(scene):
    validate_matrix({'schemaVersion': 1, 'scenes': [scene]})
    if scene.get('asrLanguage') not in ('auto', 'en', 'ja', 'zh') or scene.get('unit') not in ('word', 'char'):
        raise BatchError('scene_parameters_invalid')
    if type(scene.get('expectedSpeech')) is not bool:
        raise BatchError('speech_expectation_required')
    provenance = scene.get('provenance', {})
    if not isinstance(provenance, dict) or provenance.get('license') not in LICENSES:
        raise BatchError('public_provenance_required')
    url = provenance.get('sourceUrl')
    if not isinstance(url, str) or not url.startswith('https://') or len(url) > 2048:
        raise BatchError('public_provenance_required')
    tags = scene.get('tags', [])
    if not isinstance(tags, list) or len(tags) > 16 or any(not isinstance(tag, str) or len(tag) > 96 for tag in tags):
        raise BatchError('scene_tags_invalid')
    try:
        if Path(scene['pcmPath']).suffix not in ('.pcm', '.s16le') or Path(scene['wavPath']).suffix != '.wav':
            raise BatchError('input_extension_invalid')
        pcm = bounded_regular(scene['pcmPath'], MAX_PCM_BYTES)
        wav_bytes = bounded_regular(scene['wavPath'], MAX_PCM_BYTES + 64 * 1024)
        if not pcm or len(pcm) % 2:
            raise BatchError('pcm_invalid')
        import io
        with wave.open(io.BytesIO(wav_bytes), 'rb') as audition:
            if (audition.getnchannels(), audition.getsampwidth(), audition.getframerate(), audition.getcomptype()) != (1, 2, 16000, 'NONE'):
                raise BatchError('wav_format_invalid')
            if audition.readframes(audition.getnframes()) != pcm:
                raise BatchError('pcm_wav_mismatch')
        reference = scene.get('referencePath')
        if scene['expectedSpeech']:
            if not isinstance(reference, str) or Path(reference).suffix != '.txt':
                raise BatchError('speech_reference_required')
            reference_bytes = bounded_regular(reference, MAX_REFERENCE_BYTES)
            if not reference_bytes.decode('utf-8').strip():
                raise BatchError('speech_reference_empty')
        else:
            if reference is not None:
                raise BatchError('negative_reference_must_be_null')
            reference_bytes = None
    except (KeyError, TypeError, ValueError, EOFError, wave.Error, UnicodeDecodeError):
        raise BatchError('scene_input_invalid') from None
    return {**scene, 'pcmBytes': pcm, 'wavBytes': wav_bytes, 'referenceBytes': reference_bytes,
            'durationSeconds': len(pcm) / 32000,
            'pcmSha256': hashlib.sha256(pcm).hexdigest(),
            'wavSha256': hashlib.sha256(wav_bytes).hexdigest()}


def private_write(path, data):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'wb') as output:
        output.write(data)


def private_json(path, value):
    path = Path(path)
    temporary = path.with_name(path.name + '-' + uuid.uuid4().hex + '.tmp')
    private_write(temporary, (json.dumps(value, ensure_ascii=False, indent=2) + '\n').encode())
    os.replace(temporary, path)


def stage_scene(scene, directory):
    directory = Path(directory)
    directory.mkdir(parents=True, mode=0o700)
    private_write(directory / 'input.pcm', scene['pcmBytes'])
    private_write(directory / 'input.wav', scene['wavBytes'])
    reference = directory / 'reference.txt' if scene['referenceBytes'] is not None else None
    if reference is not None:
        private_write(reference, scene['referenceBytes'])
    manifest = directory / 'manifest.json'
    private_json(manifest, {'clips': [{'label': scene['label'], 'pcm_path': str(directory / 'input.pcm'),
                  'reference_path': str(reference) if reference else None,
                  'language': scene['asrLanguage'], 'unit': scene['unit']}]})
    metadata = {k: v for k, v in scene.items() if k not in ('pcmBytes', 'wavBytes', 'referenceBytes')}
    private_json(directory / 'scene.json', metadata)
    return {**metadata, 'manifestPath': manifest, 'outputDirectory': directory / 'evidence'}


def child_command(executable, scene, inherited):
    # The test's private dev loader reads the key itself. Never pass key/token
    # variables or an ambient arm/denoising override into an experiment.
    environment = {key: value for key, value in inherited.items() if key in
                   ('PATH', 'HOME', 'LANG', 'LC_ALL', 'LC_CTYPE', 'TMPDIR',
                    'DYLD_LIBRARY_PATH', 'MIMI_ASR_BENCH_CONFIG_DIR')}
    environment.update({'MIMI_ASR_BENCH_MANIFEST': str(scene['manifestPath']),
                        'MIMI_ASR_BENCH_PRIVATE_OUTPUT_DIR': str(scene['outputDirectory']),
                        'MIMI_ASR_BENCH_ARM': 'baseline', 'MIMI_ASR_BENCH_DENOISE': 'bypass'})
    return [str(executable), TEST_NAME, '--ignored', '--exact', '--nocapture'], environment


def run_jobs(scenes, jobs, execute, progress, cancelled=None):
    if not 1 <= jobs <= 4:
        raise BatchError('job_limit_invalid')
    cancelled = cancelled or threading.Event()
    rows = [{'label': scene['label'], 'status': 'queued'} for scene in scenes]
    progress(rows)
    next_index = 0
    pending = {}
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        try:
            while pending or next_index < len(scenes):
                while not cancelled.is_set() and next_index < len(scenes) and len(pending) < jobs:
                    index = next_index
                    next_index += 1
                    rows[index]['status'] = 'running'
                    pending[pool.submit(execute, scenes[index], cancelled)] = index
                    progress(rows)
                if cancelled.is_set():
                    for index in range(next_index, len(scenes)):
                        rows[index]['status'] = 'cancelled'
                    next_index = len(scenes)
                if not pending:
                    break
                completed, _ = wait(pending, timeout=0.1, return_when=FIRST_COMPLETED)
                for future in completed:
                    index = pending.pop(future)
                    try:
                        rows[index] = future.result()
                    except BatchError as error:
                        rows[index] = {'label': scenes[index]['label'], 'status': 'failed', 'failure': str(error)}
                    except Exception:
                        rows[index] = {'label': scenes[index]['label'], 'status': 'failed', 'failure': 'job_internal_failure'}
                    progress(rows)
        except KeyboardInterrupt:
            cancelled.set()
            for future, index in pending.items():
                try:
                    rows[index] = future.result(timeout=8)
                except Exception:
                    rows[index] = {'label': scenes[index]['label'], 'status': 'cancelled'}
            for index in range(next_index, len(scenes)):
                rows[index]['status'] = 'cancelled'
            progress(rows)
    return rows


def terminate(process):
    process.terminate()
    try:
        process.wait(timeout=2)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=2)


def compile_tests(repository, output):
    command = ['cargo', 'test', '--locked', '--manifest-path', str(repository / 'src-tauri/Cargo.toml'),
               '--features', 'development-debugger', '--lib', '--no-run', '--message-format=json']
    executable = None
    with (output / 'compile.jsonl').open('xb') as log:
        os.chmod(output / 'compile.jsonl', 0o600)
        process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, cwd=repository)
        started = time.monotonic()
        try:
            while process.poll() is None:
                if time.monotonic() - started > 600 or (output / 'compile.jsonl').stat().st_size > 8 * 1024 * 1024:
                    raise BatchError('compile_timeout_or_log_limit')
                time.sleep(0.1)
            if process.returncode != 0:
                raise BatchError('compile_failed')
            for line in bounded_regular(output / 'compile.jsonl', 8 * 1024 * 1024).splitlines():
                if len(line) > MAX_MATRIX_BYTES:
                    raise BatchError('compile_log_limit')
                try:
                    item = json.loads(line)
                except (UnicodeDecodeError, ValueError):
                    continue
                if (item.get('reason') == 'compiler-artifact' and item.get('profile', {}).get('test') and
                        item.get('executable') and item.get('target', {}).get('name') == 'mimi_lib' and
                        set(item.get('target', {}).get('kind', [])) & {'lib', 'rlib'}):
                    executable = Path(item['executable'])
            if executable is None:
                raise BatchError('compile_failed')
        except BaseException:
            if process.poll() is None:
                terminate(process)
            raise
    return executable


def execute_scene(executable, scene, cancelled):
    args, environment = child_command(executable, scene, os.environ)
    log_path = scene['manifestPath'].parent / 'job.log'
    start = time.monotonic()
    with log_path.open('xb') as log:
        os.chmod(log_path, 0o600)
        process = subprocess.Popen(args, env=environment, stdout=log, stderr=subprocess.STDOUT)
        while process.poll() is None:
            if cancelled.is_set():
                terminate(process)
                return {'label': scene['label'], 'status': 'cancelled'}
            if time.monotonic() - start > scene['durationSeconds'] + 45 or log_path.stat().st_size > MAX_MATRIX_BYTES:
                terminate(process)
                raise BatchError('job_timeout_or_log_limit')
            time.sleep(0.1)
    if process.returncode != 0:
        raise BatchError('provider_job_failed')
    directories = list(scene['outputDirectory'].glob('*/metrics.json'))
    if len(directories) != 1:
        raise BatchError('job_evidence_missing')
    metrics_path = directories[0]
    metrics = json.loads(bounded_regular(metrics_path, MAX_MATRIX_BYTES))
    if metrics.get('failure') is not None:
        raise BatchError('provider_job_failed')
    events_path = metrics_path.with_name('events.jsonl')
    request = json.loads(bounded_regular(metrics_path.with_name('request.json'), MAX_MATRIX_BYTES))
    model = request.get('payload', {}).get('model')
    if not isinstance(model, str):
        raise BatchError('job_request_evidence_invalid')
    observed_final = 0
    observed_characters = 0
    observed_lexical_final = 0
    observed_lexical_draft = 0
    for line in bounded_regular(events_path, 8 * 1024 * 1024).splitlines():
        event = json.loads(line)
        if event.get('kind') == 'transcription' and isinstance(event.get('text'), str):
            lexical = any(unicodedata.category(char)[0] in 'LNM' for char in event['text'])
            if event.get('isFinal'):
                observed_final += 1
                observed_characters += len(event['text'])
                observed_lexical_final += lexical
            else:
                observed_lexical_draft += lexical
    return {'label': scene['label'], 'status': 'complete', 'baseLabel': scene['label'],
            'provider': 'alibabaCloud', 'model': model, 'language': scene['asrLanguage'],
            'inputProcessing': 'bypass', 'durationSeconds': scene['durationSeconds'], 'pcmSha256': scene['pcmSha256'],
            'pcmPath': str(scene['manifestPath'].parent / 'input.pcm'),
            'requestPath': str(metrics_path.with_name('request.json')), 'eventsPath': str(events_path),
            'metricsPath': str(metrics_path), 'expectedSpeech': scene['expectedSpeech'],
            'observedFinalEvents': observed_final, 'observedFinalCharacters': observed_characters,
            'observedLexicalFinalEvents': observed_lexical_final, 'observedLexicalDraftEvents': observed_lexical_draft,
            'unexpectedTranscription': not scene['expectedSpeech'] and observed_lexical_final > 0,
            'elapsedSeconds': round(time.monotonic() - start, 3)}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', required=True, type=Path)
    parser.add_argument('--output-root', required=True, type=Path)
    parser.add_argument('--jobs', type=int, default=3)
    parser.add_argument('--run', action='store_true', help='Explicitly execute paid provider requests after validation.')
    args = parser.parse_args(argv)
    if not 1 <= args.jobs <= 4:
        raise BatchError('job_limit_invalid')
    output_root = args.output_root
    if not output_root.is_absolute() or '..' in output_root.parts or not output_root.is_relative_to(OUTPUT_PARENT):
        raise BatchError('output_root_invalid')
    matrix = json.loads(bounded_regular(args.manifest, MAX_MATRIX_BYTES))
    prepared = [prepare_scene(scene) for scene in validate_matrix(matrix)]
    if sum(len(scene['pcmBytes']) + len(scene['wavBytes']) for scene in prepared) > MAX_STAGED_BYTES:
        raise BatchError('batch_input_size_limit')
    output_root.mkdir(parents=True, exist_ok=True, mode=0o700)
    if output_root.is_symlink() or not output_root.resolve().is_relative_to(OUTPUT_PARENT):
        raise BatchError('output_root_invalid')
    output = output_root / ('batch-' + uuid.uuid4().hex)
    output.mkdir(mode=0o700)
    staged = [stage_scene(scene, output / scene['label']) for scene in prepared]
    private_json(output / 'matrix.json', matrix)
    private_json(output / 'progress.json', {'status': 'prepared', 'jobs': args.jobs,
                 'rows': [{'label': scene['label'], 'status': 'prepared'} for scene in staged]})
    print(json.dumps({'status': 'prepared', 'scenes': len(staged), 'jobs': args.jobs}), flush=True)
    if not args.run:
        return 0
    if sys.platform != 'darwin':
        raise BatchError('paid_batch_requires_macos_dev_credentials')
    import fcntl
    lock_path = OUTPUT_PARENT / '.provider-batch.lock'
    fd = os.open(lock_path, os.O_CREAT | os.O_RDWR | getattr(os, 'O_NOFOLLOW', 0), 0o600)
    with os.fdopen(fd, 'a') as lease:
        try:
            fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise BatchError('another_provider_batch_running') from None
        private_json(output / 'progress.json', {'status': 'compiling', 'jobs': args.jobs,
                     'rows': [{'label': scene['label'], 'status': 'queued'} for scene in staged]})
        try:
            repository = Path(__file__).resolve().parents[1]
            executable = compile_tests(repository, output)
            revision = subprocess.run(['git', 'rev-parse', 'HEAD'], cwd=repository, capture_output=True,
                                      check=True, timeout=5).stdout.decode().strip()
            dirty = bool(subprocess.run(['git', 'status', '--porcelain'], cwd=repository, capture_output=True,
                                        check=True, timeout=5).stdout)
            digest = hashlib.sha256()
            with executable.open('rb') as binary:
                for chunk in iter(lambda: binary.read(1024 * 1024), b''):
                    digest.update(chunk)
            private_json(output / 'build.json', {'repositoryRevision': revision, 'workingTreeDirty': dirty,
                         'testExecutableSha256': digest.hexdigest(), 'workers': args.jobs,
                         'arm': 'baseline', 'inputProcessing': 'bypass',
                         'limitation': 'A dirty checkout is not identical to the recorded commit. Binary hash identifies the actual compiled test executable.'})
        except BatchError as error:
            private_json(output / 'progress.json', {'status': 'failed', 'failure': str(error), 'rows': []})
            raise

        def progress(rows):
            private_json(output / 'progress.json', {'status': 'running', 'jobs': args.jobs, 'rows': rows})
            private_json(output / 'baseline-index.json', {'schemaVersion': 1,
                         'clips': [row for row in rows if row['status'] == 'complete']})

        rows = run_jobs(staged, args.jobs, lambda scene, signal: execute_scene(executable, scene, signal), progress)
        status = 'complete' if all(row['status'] == 'complete' for row in rows) else 'incomplete'
        private_json(output / 'progress.json', {'status': status, 'jobs': args.jobs, 'rows': rows})
        print(json.dumps({'status': status, 'scenes': len(rows),
                          'completed': sum(row['status'] == 'complete' for row in rows),
                          'failed': sum(row['status'] == 'failed' for row in rows),
                          'unexpectedSpeechCases': sum(row.get('unexpectedTranscription', False) for row in rows)}), flush=True)
        return 0 if status == 'complete' else 1


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (BatchError, OSError, ValueError) as error:
        failure = str(error) if isinstance(error, BatchError) else 'batch_internal_failure'
        print(json.dumps({'status': 'failed', 'failure': failure}), flush=True)
        raise SystemExit(2)
