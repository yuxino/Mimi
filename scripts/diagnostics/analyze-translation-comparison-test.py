#!/usr/bin/env python3
"""Offline metric contracts; no provider calls or credentials."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('comparison', Path(__file__).with_name('analyze-translation-comparison.py'))
comparison = importlib.util.module_from_spec(spec)
spec.loader.exec_module(comparison)


def event(at, text, kind='translation_preview', stream='one', end=None):
    return {'at_ms': at, 'kind': kind, 'stream_id': stream, 'text': text, 'speech_end_ms': end}


class ComparisonTests(unittest.TestCase):
    def trial(self, events):
        return comparison.summarize_trial({'model': 'synthetic', 'observations': events, 'failure': None})

    def test_extension_is_not_a_rewrite_and_stability_is_retrospective(self):
        result = self.trial([event(100, 'hello'), event(200, 'hello world'),
                             event(300, 'hello there'), event(400, 'hello world'),
                             event(500, 'hello world', 'translation_final', end=350)])
        row = result['streams'][0]
        self.assertEqual(result['firstTranslationAtMs'], 100)
        self.assertEqual(row['rewrites'], 2)
        self.assertEqual(row['stableTextAtMs'], 400)
        self.assertEqual(row['stableWaitFromFirstTextMs'], 300)
        self.assertEqual(row['confirmationLagAfterSpeechMs'], 150)

    def test_response_identity_separates_new_sentences_from_rewrites(self):
        result = self.trial([event(100, 'one', stream='a'), event(200, 'two', stream='b'),
                             event(300, 'two', 'translation_final', 'b')])
        self.assertEqual([row['rewrites'] for row in result['streams']], [0, 0])
        self.assertIsNone(result['streams'][0]['stableTextAtMs'])
        self.assertIsNone(result['streams'][1]['confirmationLagAfterSpeechMs'])

    def test_empty_or_unconfirmed_stream_cannot_pass_quality(self):
        result = self.trial([event(100, ''), event(200, 'draft')])
        self.assertIsNone(result['streams'][0]['stableTextAtMs'])
        self.assertEqual(result['semanticReview']['readability'], 'pending')

    def test_reordered_clock_and_missing_identity_fail(self):
        for events in ([event(200, 'a'), event(100, 'b')],
                       [dict(event(100, 'a'), stream_id=None)]):
            with self.assertRaises(comparison.batch.BatchError):
                self.trial(events)


if __name__ == '__main__':
    unittest.main()
