#!/usr/bin/env python3
"""Standard-library-only tests for the offline research probes."""
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
import wave


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(file))
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


vad = load("mt_vad_research", "mt-vad-benchmark.py")
policy = load("mt_unit_research", "mt-unit-policy.py")


class ActivityTests(unittest.TestCase):
    def test_short_impulse_does_not_start_and_silence_requires_confirmation(self):
        gate = vad.ActivityGate(400)
        self.assertFalse(any(gate.push(0.9) for _ in range(5)))
        self.assertFalse(gate.push(0.0))
        for _ in range(8):
            self.assertFalse(gate.push(0.9))
        self.assertTrue(gate.push(0.9))
        for _ in range(13):
            self.assertTrue(gate.push(0.0))
        self.assertFalse(gate.push(0.0))

    def test_voice_return_cancels_pending_end(self):
        gate = vad.ActivityGate(400)
        for _ in range(12):
            gate.push(0.9)
        for _ in range(12):
            self.assertTrue(gate.push(0.0))
        self.assertTrue(gate.push(0.9))
        self.assertTrue(gate.push(0.0))

    def test_wave_validation_is_bounded_and_hashes_pcm(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "sample.wav"
            with wave.open(str(path), "wb") as out:
                out.setparams((1, 2, 16000, 0, "NONE", "NONE"))
                out.writeframes(b"\0" * 1024)
            self.assertEqual(vad.validate_wave(path)[0], 512)
            content = path.read_bytes()
            path.write_bytes(content[:-2])
            with self.assertRaisesRegex(ValueError, "truncated"):
                vad.validate_wave(path)

    def test_invalid_format_and_duplicate_labels_rejected_before_inference(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "sample.wav"
            with wave.open(str(path), "wb") as out:
                out.setparams((2, 2, 48000, 0, "NONE", "NONE"))
                out.writeframes(b"\0" * 1024)
            with self.assertRaisesRegex(ValueError, "PCM16"):
                vad.validate_wave(path)
            with wave.open(str(path), "wb") as out:
                out.setparams((1, 2, 16000, 0, "NONE", "NONE"))
                out.writeframes(b"\0" * 1024)
            scene = {"label": "synthetic", "wavPath": "sample.wav", "provenance": {
                "license": "synthetic", "sourceUrl": "generated:zero"}}
            manifest = Path(directory) / "matrix.json"
            manifest.write_text(json.dumps({"scenes": [scene, scene]}), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "unique"):
                vad.load_matrix(manifest)


class UnitTests(unittest.TestCase):
    def test_stable_unit_excludes_incomplete_tail_and_next_request_only_has_suffix(self):
        p = policy.UnitPolicy()
        p.observe(0, "First sentence. I want")
        self.assertIsNone(p.next(599))
        request = p.next(600)
        self.assertEqual(request.suffix, "First sentence.")
        self.assertTrue(p.complete(700, request, nonempty_success=True))
        p.observe(900, "First sentence. I want to stay.")
        request = p.next(2100)
        self.assertEqual(request.suffix, "I want to stay.")

    def test_pause_does_not_freeze_japanese_negation_or_chinese_incomplete_clause(self):
        for text in ["私は行くつもりでは", "如果明天下雨，我们就"]:
            p = policy.UnitPolicy()
            p.observe(0, text)
            request = p.next(600, paused=True)
            self.assertEqual(request.kind, "replaceable-preview")
            self.assertTrue(p.complete(650, request, nonempty_success=True))
            self.assertEqual(p.pinned, "")

    def test_continuous_unpunctuated_speech_has_bounded_preview_wait(self):
        p = policy.UnitPolicy()
        for at in range(0, 2001, 100):
            p.observe(at, "word " * (at // 100 + 1))
        request = p.next(2000)
        self.assertEqual(request.kind, "replaceable-preview")

    def test_asr_correction_explicitly_revokes_pinned_prefix(self):
        p = policy.UnitPolicy()
        p.observe(0, "I can go. More")
        request = p.next(600)
        p.complete(700, request, nonempty_success=True)
        p.observe(800, "I cannot go. More")
        self.assertEqual(p.pinned, "")
        self.assertEqual(p.invalidations, 1)

    def test_late_response_after_correction_reset_and_provider_final_is_rejected(self):
        for transition in ["correction", "reset", "final"]:
            p = policy.UnitPolicy()
            p.observe(0, "I can go.")
            request = p.next(600)
            if transition == "correction":
                p.observe(650, "I cannot go.")
            elif transition == "reset":
                p.reset()
            else:
                self.assertEqual(p.provider_final(650, "I cannot go."), "I cannot go.")
            self.assertFalse(p.complete(700, request, nonempty_success=True))
            self.assertEqual(p.pinned, "")

    def test_empty_or_failed_translation_never_advances_boundary_and_can_retry(self):
        p = policy.UnitPolicy()
        p.observe(0, "First sentence.")
        request = p.next(600)
        self.assertFalse(p.complete(700, request, nonempty_success=False))
        self.assertEqual(p.pinned, "")
        self.assertIsNone(p.next(2099))
        self.assertIsNotNone(p.next(2100))

    def test_hung_request_is_bounded_and_cannot_publish_after_replacement(self):
        p = policy.UnitPolicy()
        p.observe(0, "First sentence.")
        request = p.next(600)
        replacement = p.next(12600)
        self.assertIsNotNone(replacement)
        self.assertFalse(p.complete(12601, request, nonempty_success=True))
        self.assertTrue(p.complete(12602, replacement, nonempty_success=True))

    def test_reset_does_not_refund_request_spacing(self):
        p = policy.UnitPolicy()
        p.observe(0, "First sentence.")
        p.next(600)
        p.reset()
        p.observe(700, "Another sentence.")
        self.assertIsNone(p.next(1300))
        self.assertIsNotNone(p.next(2100))

    def test_same_text_after_provider_final_is_a_new_utterance(self):
        p = policy.UnitPolicy()
        p.observe(0, "Again.")
        request = p.next(600)
        p.complete(700, request, nonempty_success=True)
        p.provider_final(800, "Again.")
        p.observe(900, "Again.")
        self.assertEqual(p.next(2100).suffix, "Again.")

    def test_sources_have_independent_state(self):
        system, microphone = policy.UnitPolicy(), policy.UnitPolicy()
        system.observe(0, "System sentence.")
        request = system.next(600)
        system.complete(700, request, nonempty_success=True)
        microphone.observe(700, "Microphone sentence.")
        self.assertEqual(system.pinned, "System sentence.")
        self.assertEqual(microphone.pinned, "")

    def test_dedup_and_text_limit(self):
        p = policy.UnitPolicy()
        p.observe(0, "unfinished phrase")
        request = p.next(2000)
        p.complete(2100, request, nonempty_success=True)
        self.assertIsNone(p.next(4000))
        with self.assertRaisesRegex(ValueError, "oversized"):
            p.observe(4100, "字" * policy.MAX_TEXT_BYTES)
        with self.assertRaisesRegex(ValueError, "non-monotonic"):
            p.next(1)

    def test_maximum_wait_preview_does_not_prevent_later_stable_candidate(self):
        p = policy.UnitPolicy()
        p.observe(0, "Nearly finished")
        p.observe(1950, "Nearly finished.")
        preview = p.next(2000)
        self.assertEqual(preview.kind, "replaceable-preview")
        p.complete(2100, preview, nonempty_success=True)
        unit = p.next(3500)
        self.assertEqual(unit.kind, "provisional-unit")
        self.assertTrue(p.complete(3600, unit, nonempty_success=True))
        self.assertEqual(p.pinned, "Nearly finished.")

    def test_decimal_abbreviation_ellipsis_and_quoted_cjk_boundary(self):
        for source in ["Dr. Smith", "It costs 3.14", "U.S. policy", "Wait... maybe"]:
            self.assertEqual(policy.complete_prefix(source), "")
        self.assertEqual(policy.complete_prefix("「完了。」次は"), "「完了。」")

    def test_empty_draft_does_not_revoke_readable_prefix(self):
        p = policy.UnitPolicy()
        p.observe(0, "Readable sentence.")
        request = p.next(600)
        p.complete(700, request, nonempty_success=True)
        p.observe(800, " ")
        self.assertEqual(p.pinned, "Readable sentence.")
        self.assertEqual(p.invalidations, 0)

    def test_punctuation_only_draft_never_requests_translation(self):
        p = policy.UnitPolicy()
        p.observe(0, "……！！")
        self.assertIsNone(p.next(2000, paused=True))


if __name__ == "__main__":
    unittest.main()
