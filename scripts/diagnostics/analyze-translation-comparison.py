#!/usr/bin/env python3
"""Offline analysis of the opted-in translation probe's private event ledger.

Never opens credentials, calls services, or infers semantic quality from counts.
The input/output contain the explicitly supplied test sample's subtitles; keep
both outside Git. Metrics use the first scheduled PCM frame as time zero.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import sys

spec = importlib.util.spec_from_file_location('batch', Path(__file__).resolve().parents[1] / 'development/run-development-batch.py')
batch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(batch)

MAX_EVENTS = 20000
MAX_TEXT = 65536
KINDS = {'source_preview', 'source_final', 'translation_preview', 'translation_final', 'finished'}


def summarize_trial(trial):
    if not isinstance(trial, dict) or not isinstance(trial.get('model'), str):
        raise batch.BatchError('comparison_invalid')
    events = trial.get('observations')
    if not isinstance(events, list) or len(events) > MAX_EVENTS:
        raise batch.BatchError('comparison_invalid')
    streams = {}
    previous_at = 0
    first_translation = None
    for event in events:
        if not isinstance(event, dict):
            raise batch.BatchError('event_invalid')
        at = event.get('at_ms')
        kind = event.get('kind')
        text = event.get('text')
        stream = event.get('stream_id')
        if (type(at) is not int or not previous_at <= at <= 120000 or kind not in KINDS
                or not isinstance(text, str) or len(text.encode()) > MAX_TEXT
                or not isinstance(stream, str) or len(stream) > 512):
            raise batch.BatchError('event_invalid')
        previous_at = at
        if kind == 'finished':
            continue
        lane = 'source' if kind.startswith('source_') else 'translation'
        key = (lane, stream)
        row = streams.setdefault(key, {'lane': lane, 'streamId': stream, 'updates': [],
                                      'rewrites': 0, 'finalAtMs': None, 'finalText': None,
                                      'speechEndMs': None})
        if row['updates'] and text != row['updates'][-1][1] and not text.startswith(row['updates'][-1][1]):
            row['rewrites'] += 1
        row['updates'].append((at, text))
        end = event.get('speech_end_ms')
        if end is not None:
            if type(end) is not int or end < 0 or end > 60000:
                raise batch.BatchError('event_invalid')
            row['speechEndMs'] = end
        if lane == 'translation' and text.strip() and first_translation is None:
            first_translation = at
        if kind.endswith('_final'):
            row['finalAtMs'] = at
            row['finalText'] = text
    output = []
    for row in streams.values():
        stable = None
        final = row['finalText']
        # Retrospective: the final text first appeared here and never changed
        # afterward. An equal earlier draft followed by a rewrite is not stable.
        if final is not None:
            for at, text in reversed(row['updates']):
                if text != final:
                    break
                stable = at
        first = next((at for at, text in row['updates'] if text.strip()), None)
        output.append({k: v for k, v in row.items() if k != 'updates'} | {
            'firstTextAtMs': first,
            'stableTextAtMs': stable,
            'stableWaitFromFirstTextMs': stable - first if stable is not None and first is not None else None,
            'confirmationLagAfterSpeechMs': row['finalAtMs'] - row['speechEndMs']
                if row['finalAtMs'] is not None and row['speechEndMs'] is not None else None,
            'updates': len(row['updates']),
        })
    return {'model': trial['model'], 'failure': trial.get('failure'),
            'firstTranslationAtMs': first_translation, 'streams': output,
            'semanticReview': {'omissions': 'pending', 'readability': 'pending',
                               'answersInsteadOfTranslation': 'pending', 'interruptions': 'pending'}}


def analyze(value):
    trials = value.get('trials') if isinstance(value, dict) else None
    if not isinstance(trials, list) or not 1 <= len(trials) <= 3:
        raise batch.BatchError('comparison_invalid')
    return {'schemaVersion': 1,
            'timingOrigin': 'first scheduled PCM frame; connection setup excluded',
            'evidenceBoundary': 'provider/runtime observations; no native visible-caption acceptance',
            'trials': [summarize_trial(trial) for trial in trials]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    try:
        if not args.output.is_absolute() or args.output.exists():
            raise batch.BatchError('output_must_be_new_absolute_file')
        result = analyze(json.loads(batch.bounded_regular(args.input, 3 * 1024 * 1024)))
        batch.private_write(args.output, (json.dumps(result, ensure_ascii=False, indent=2) + '\n').encode())
        print('Translation comparison analyzed locally; semantic review remains pending.')
    except (batch.BatchError, OSError, UnicodeError, json.JSONDecodeError):
        print('Translation comparison analysis failed; input or output invalid.', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
