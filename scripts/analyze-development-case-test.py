#!/usr/bin/env python3
"""Focused regression checks for private, evidence-bound case analysis."""

import importlib.util
import io
import json
from pathlib import Path
import stat
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from unittest.mock import patch


sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location(
    "analyze_development_case", Path(__file__).with_name("analyze-development-case.py")
)
ANALYZE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ANALYZE)


def cue(text, number=1):
    return {"sentence_id": number, "text": text, "timing_basis": "untimed_reference"}


def provider(kind="confirmedPair", producer="provider", admission="queueReliableAttempt",
             sequence=10, source="system", revision=0, text="A small example."):
    return {"elapsedMs": 200, "event": {"kind": "provider", "observation": {
        "producer": producer, "admission": admission, "transportSequence": sequence,
        "source": source, "generation": 2, "contentRevision": revision,
        "eventKind": kind, "utteranceId": 8,
    }, "content": {"source": text, "translation": "Synthetic translation"}}}


def decision(sequence=10, revision=0, admission="accepted", source="system"):
    row = provider(producer="session", admission=admission, sequence=sequence,
                   revision=revision, source=source)
    return {"id": 42, "elapsedMs": 205, "event": row["event"]}


class CaseAnalysisTests(unittest.TestCase):
    def test_final_sentence_revision_replaces_draft_and_old_final(self):
        rows = [
            {"kind": "transcription", "isFinal": False, "sentenceId": 1, "text": "draft"},
            {"kind": "transcription", "isFinal": True, "sentenceId": 1, "text": "old final"},
            {"kind": "transcription", "isFinal": True, "sentenceId": 2, "text": "second"},
            {"kind": "transcription", "isFinal": True, "sentenceId": 1, "text": "corrected final"},
        ]
        finals, counts = ANALYZE.asr_finals(rows)
        self.assertEqual([item["text"] for item in finals], ["corrected final", "second"])
        self.assertEqual(counts["finalRevisions"], 1)

    def test_missing_sentence_identity_preserves_real_repeated_finals(self):
        finals, counts = ANALYZE.asr_finals([
            {"kind": "transcription", "isFinal": True, "text": "Again."},
            {"kind": "transcription", "isFinal": True, "text": "Again."},
        ])
        self.assertEqual(len(finals), 2)
        self.assertEqual(counts["missingSentenceIds"], 2)

    def test_unicode_normalization_does_not_rewrite_numbers(self):
        self.assertEqual(ANALYZE.normalize("ＨＥＬＬＯ—world! I’m here.", "word"),
                         ["hello", "world", "i'm", "here"])
        self.assertEqual(ANALYZE.normalize("你好，世界！１２", "char"), list("你好世界12"))
        self.assertNotEqual(ANALYZE.normalize("twenty four", "word"), ANALYZE.normalize("24", "word"))

    def test_internal_letter_hyphen_is_not_a_missing_word_or_a_numeric_rewrite(self):
        self.assertEqual(ANALYZE.normalize("Super-G twenty-fourth", "word"),
                         ANALYZE.normalize("superG twentyfourth", "word"))
        self.assertNotEqual(ANALYZE.normalize("twenty-fourth", "word"), ANALYZE.normalize("24th", "word"))
        self.assertEqual(ANALYZE.normalize("3-5", "word"), ["3", "5"])
        score = ANALYZE.compare([cue("Super-G")], "superG", "word")
        self.assertEqual((score["deletions"], score["substitutions"]), (0, 0))

    def test_edit_alignment_locates_partial_and_missing_cues(self):
        score = ANALYZE.compare([cue("red car", 1), cue("the blue book", 2)], "red van", "word")
        self.assertEqual((score["deletions"], score["insertions"], score["substitutions"]), (3, 0, 1))
        self.assertEqual([item["status"] for item in score["cues"]], ["partial", "unmatched"])
        self.assertEqual(score["cues"][1]["startSeconds"], None)

    def test_reference_repetition_is_not_deduplicated(self):
        score = ANALYZE.compare([cue("again now", 1), cue("again now", 2)], "again now", "word")
        self.assertEqual(score["referenceUnits"], 4)
        self.assertEqual(score["deletions"], 2)
        self.assertEqual(sorted(item["status"] for item in score["cues"]), ["matched", "unmatched"])

    def test_insertion_and_substitution_counts_are_not_only_deletion_recall(self):
        score = ANALYZE.compare([cue("one blue book")], "extra one red book", "word")
        self.assertEqual((score["matches"], score["deletions"], score["insertions"], score["substitutions"]),
                         (2, 0, 1, 1))
        self.assertEqual(score["errorRate"], 2 / 3)

    def test_alignment_cap_returns_unknown_instead_of_partial_score(self):
        with patch.object(ANALYZE, "MAX_ALIGNMENT_CELLS", 4):
            score = ANALYZE.compare([cue("one two")], "one two", "word")
        self.assertEqual(score["status"], "alignment_limit")
        self.assertIsNone(score["errorRate"])

    def test_exact_admission_join_requires_sequence_revision_and_source(self):
        _, pairs, _ = ANALYZE.provider_stages(
            [provider()], [decision(sequence=11), decision(revision=1), decision(source="microphone")]
        )
        self.assertEqual(pairs[0]["acceptance"], "unknown")
        _, pairs, _ = ANALYZE.provider_stages([provider()], [decision()])
        self.assertEqual(pairs[0]["acceptance"], "accepted")
        self.assertEqual(pairs[0]["matchedTraceEvents"][0]["traceEventId"], 42)

    def test_old_missing_sequence_never_becomes_accepted_by_owner_or_count(self):
        _, pairs, _ = ANALYZE.provider_stages([provider(sequence=None)], [decision(sequence=None)])
        self.assertEqual(pairs[0]["acceptance"], "unknown")

    def test_conflicting_admissions_are_unknown_and_rejection_is_not_acceptance(self):
        _, pairs, _ = ANALYZE.provider_stages([provider()], [decision(), decision(admission="paused")])
        self.assertEqual(pairs[0]["acceptance"], "unknown")
        _, pairs, _ = ANALYZE.provider_stages([provider()], [decision(admission="staleContent")])
        self.assertEqual(pairs[0]["acceptance"], "rejected")

    def test_recognition_final_revision_is_source_bound(self):
        rows = [provider("sourceFinal", "recognition", text="old"),
                provider("sourceFinal", "recognition", text="updated"),
                provider("sourceFinal", "recognition", source="microphone", text="other")]
        raw, _, _ = ANALYZE.provider_stages(rows, [])
        self.assertEqual([item["text"] for item in raw], ["updated", "other"])

    def test_missing_private_text_is_unknown_evidence_not_empty_recognition(self):
        row = provider("sourceFinal", "recognition")
        row["event"]["content"] = None
        raw, _, _ = ANALYZE.provider_stages([row], [])
        self.assertFalse(raw[0]["textEvidenceAvailable"])
        row["event"]["content"] = {"role": "source", "text": "custom recognition", "isFinal": True}
        raw, _, _ = ANALYZE.provider_stages([row], [])
        self.assertEqual(raw[0]["text"], "custom recognition")

    def test_history_identity_keeps_repeated_text_and_latest_revision(self):
        first = {"audioSource": "system", "createdAt": 100, "source": "old", "translation": "first"}
        corrected = {**first, "source": "repeat", "translation": "corrected"}
        repeat = {**corrected, "createdAt": 200}
        microphone = {**corrected, "audioSource": "microphone"}
        snapshots = [
            {"snapshot": {"subtitles": {"history": [first], "tracks": [{"audioSource": "system", "history": [first]}]}}},
            {"snapshot": {"debugSnapshotId": 9, "subtitles": {"history": [corrected, repeat, microphone],
                "tracks": [{"audioSource": "system", "history": [corrected, repeat]},
                           {"audioSource": "microphone", "history": [microphone]}]}}},
        ]
        history, _ = ANALYZE.history_rows(snapshots)
        self.assertEqual(len(history), 3)
        self.assertEqual([item["text"] for item in history], ["repeat"] * 3)
        self.assertEqual(history[0]["translation"], "corrected")
        self.assertEqual(history[0]["snapshotId"], 9)

    def test_missing_history_timestamp_is_not_guessed_or_text_deduplicated(self):
        history, counts = ANALYZE.history_rows([
            {"snapshot": {"subtitles": {"history": [{"source": "unknown", "translation": "unknown"}]}}}
        ])
        self.assertEqual(history, [])
        self.assertEqual(counts["missingHistoryIdentities"], 1)

    def test_history_collision_within_one_snapshot_is_unknown(self):
        first = {"audioSource": "system", "createdAt": 100, "source": "one", "translation": "first"}
        second = {**first, "source": "two"}
        history, counts = ANALYZE.history_rows([
            {"snapshot": {"subtitles": {"history": [first, second]}}},
            {"snapshot": {"subtitles": {"history": [second]}}},
        ])
        self.assertEqual(history, [])
        self.assertEqual(counts["ambiguousHistoryIdentities"], 1)

    def test_inactive_initial_history_is_excluded_without_text_deduplication(self):
        old = {"audioSource": "system", "createdAt": 100, "source": "again", "translation": "again"}
        new = {**old, "createdAt": 200}
        history, counts = ANALYZE.history_rows([
            {"snapshot": {"isActive": False, "subtitles": {"history": [old]}}},
            {"snapshot": {"isActive": True, "subtitles": {"history": [new]}}},
        ])
        self.assertEqual(len(history), 1)
        self.assertEqual(history[0]["createdAt"], 200)
        self.assertEqual(counts["preexistingHistoryIdentitiesExcluded"], 1)

    def test_frontend_geometry_does_not_create_a_text_error_score(self):
        summary = ANALYZE.frontend_summary([
            {"id": 7, "elapsedMs": 100, "event": {"kind": "frontend", "observation": {
                "window": "overlay", "stage": "overlayCommitted", "overflowedBlocks": 2,
                "viewportHeight": 50, "sourceCharacters": 120, "text": "must not copy this",
            }}},
            {"event": {"kind": "stopped", "unflushedWindows": ["overlay"]}},
        ])
        self.assertIsNone(summary["textErrorRate"])
        self.assertEqual(summary["overflowObservationCount"], 1)
        self.assertEqual(summary["unflushedWindows"], ["overlay"])
        self.assertNotIn("must not copy", json.dumps(summary))

    def test_durable_trace_restores_ring_eviction_and_sorts_by_id(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "trace-events.jsonl").write_text("\n".join(json.dumps({"id": i, "event": {}})
                for i in (3, 1, 2)) + "\n")
            rows, coverage = ANALYZE.trace_entries(root, {"tracePersistenceEnabled": True,
                "persistedTraceEntries": 3, "traceDropped": 0, "traceLimited": False, "traceFailed": False,
                "trace": {"recorded": 3, "evicted": 1, "entries": [{"id": 2}, {"id": 3}]}})
            self.assertEqual([row["id"] for row in rows], [1, 2, 3])
            self.assertTrue(coverage["complete"])
            self.assertFalse(coverage["onDiskLossOrIncomplete"])

    def test_durable_missing_id_and_disk_drop_are_reported(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "trace-events.jsonl").write_text(json.dumps({"id": 2, "event": {}}) + "\n")
            _, coverage = ANALYZE.trace_entries(root, {"tracePersistenceEnabled": True,
                "traceDropped": 1, "trace": {"recorded": 2, "entries": []}})
            self.assertEqual(coverage["missingEventIds"], 1)
            self.assertFalse(coverage["complete"])
            self.assertTrue(coverage["onDiskLossOrIncomplete"])

    def test_legacy_case_ring_eviction_is_incomplete_without_durable_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            _, coverage = ANALYZE.trace_entries(Path(temporary), {
                "trace": {"recorded": 4, "evicted": 2, "entries": [{"id": 3}, {"id": 4}]}})
            self.assertEqual(coverage["source"], "memory_ring")
            self.assertEqual(coverage["missingEventIds"], 2)
            self.assertFalse(coverage["complete"])

    def test_observed_asr_configuration_replaces_inferred_route_match(self):
        request = {"payload": {"model": "actual-model", "parameters": {"format": "pcm", "sample_rate": 16000},
                               "input": {"context": [{"role": "user", "content": "synthetic context"}]}}}
        rows = [{"event": {"kind": "asrRequest", "protocol": "audio3", "source": "system", "generation": 2,
                            "bodyLimited": False, "body": request}},
                {"event": {"kind": "asrRequest", "protocol": "audio3", "source": "microphone", "generation": 3,
                            "bodyLimited": False, "body": {"payload": {"model": "unrelated-model"}}}}]
        result = ANALYZE.configuration({"provider": "alibabaCloud", "sourceLanguage": "auto"}, request, {},
                                       {"language": "English"}, rows, [{"source": "system", "generation": 2}])
        self.assertEqual(result["modelMatch"], "observed_match")
        self.assertEqual(len(result["observedAsrRequests"]), 1)
        self.assertTrue(result["observedAsrRequests"][0]["parametersEqualToBaseline"])
        self.assertTrue(result["observedAsrRequests"][0]["contextEqualToBaseline"])
        self.assertNotIn("synthetic context", json.dumps(result))

    def test_truncated_asr_request_cannot_prove_configuration_match(self):
        rows = [{"event": {"kind": "asrRequest", "protocol": "audio3", "source": "system", "generation": 2,
                            "bodyLimited": True, "body": {"payload": {"model": "limited"}}}}]
        result = ANALYZE.configuration({}, {"payload": {"model": "limited"}}, {}, {}, rows)
        self.assertEqual(result["modelMatch"], "unknown")
        self.assertIsNone(result["observedAsrRequests"][0]["parametersEqualToBaseline"])

    def fixture(self, root):
        case, baseline, output = root / "case", root / "baseline", root / "output"
        case.mkdir(); baseline.mkdir()
        reference = root / "reference.txt"
        reference.write_text("A small example.\n")
        media = root / "media.json"
        media.write_text(json.dumps({"clips": [{"label": "synthetic", "language": "English", "unit": "word",
                                                "reference_path": str(reference), "sentences": [cue("A small example.")]}]}))
        route = {"provider": "alibabaCloud", "sourceLanguage": "auto", "audioInput": "system", "recordedContent": True}
        (case / "manifest.json").write_text(json.dumps({"id": "synthetic-case", "route": route,
                                                        "containsPrivateAudioAndSubtitles": True}))
        (case / "trace.json").write_text(json.dumps({"caseId": "synthetic-case", "route": route,
                                                     "trace": {"enabled": False, "recorded": 42, "entries": [decision()]}}))
        (case / "events.jsonl").write_text(json.dumps(provider()) + "\n")
        (case / "snapshots.jsonl").write_text(json.dumps({"snapshot": {"subtitles": {"history": [
            {"audioSource": "system", "createdAt": 100, "source": "A small example.", "translation": "Example"}]}}}) + "\n")
        (baseline / "events.jsonl").write_text(json.dumps({"kind": "transcription", "isFinal": True,
            "sentenceId": 1, "text": "A small example."}) + "\n" + json.dumps({"kind": "taskFinished"}) + "\n")
        (baseline / "request.json").write_text(json.dumps({"header": {"Authorization": "SECRET-NEVER-COPY"},
            "payload": {"model": "qwen-audio-3.0-asr-flash-streaming", "parameters": {"format": "pcm", "sample_rate": 16000}}}))
        (baseline / "metrics.json").write_text(json.dumps({"failure": None}))
        arguments = ["--case", str(case), "--media-manifest", str(media), "--clip", "synthetic",
                     "--asr-result", str(baseline), "--output-directory", str(output)]
        return case, baseline, output, arguments

    def test_cli_creates_private_reports_without_printing_text_paths_or_credentials(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            _, _, output, arguments = self.fixture(root)
            stdout, stderr = io.StringIO(), io.StringIO()
            with redirect_stdout(stdout), redirect_stderr(stderr):
                self.assertEqual(ANALYZE.main(arguments), 0)
            self.assertEqual(json.loads(stdout.getvalue())["status"], "complete")
            self.assertEqual(stderr.getvalue(), "")
            self.assertNotIn(temporary, stdout.getvalue())
            self.assertNotIn("A small example", stdout.getvalue())
            report_text = (output / "analysis.json").read_text()
            self.assertNotIn("SECRET-NEVER-COPY", report_text)
            self.assertNotIn("Authorization", report_text)
            self.assertTrue((output / "analysis.md").exists())
            if os_name() == "posix":
                self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o700)
                self.assertEqual(stat.S_IMODE((output / "analysis.json").stat().st_mode), 0o600)

    def test_inconsistent_reference_rejected_without_printing_the_private_text(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            _, _, _, arguments = self.fixture(root)
            (root / "reference.txt").write_text("PRIVATE-REFERENCE-MISMATCH")
            stdout, stderr = io.StringIO(), io.StringIO()
            with redirect_stdout(stdout), redirect_stderr(stderr):
                self.assertEqual(ANALYZE.main(arguments), 2)
            self.assertEqual(json.loads(stderr.getvalue())["status"], "reference_cues_disagree_with_transcript")
            self.assertNotIn("PRIVATE-REFERENCE", stdout.getvalue() + stderr.getvalue())
            self.assertNotIn(temporary, stdout.getvalue() + stderr.getvalue())


def os_name():
    import os
    return os.name


if __name__ == "__main__":
    unittest.main()
