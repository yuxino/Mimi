#!/usr/bin/env python3
"""Offline, private analysis of one recorded dev case and one reference clip.

No network or credential access. stdout/stderr contain fixed status labels and
counts only; transcripts and token differences belong exclusively in 0600 files.
"""

import argparse
from array import array
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import unicodedata
import wave


MAX_JSON_BYTES = 4 * 1024 * 1024
MAX_JOURNAL_BYTES = 20 * 1024 * 1024
MAX_LINE_BYTES = 512 * 1024
MAX_ALIGNMENT_CELLS = 4_000_000
MAX_AUDIO_BYTES = 20 * 1024 * 1024
NORMALIZATION = (
    "Unicode NFKC and casefold; word tokens retain internal apostrophes. "
    "Internal ASCII/Unicode hyphens between letters are removed (Super-G = superG); "
    "numeric ranges and dash-separated clauses are not joined. "
    "CER counts Unicode letter/number/mark code points, not grapheme clusters. "
    "Whitespace, external punctuation and symbols are ignored. Numbers, names, "
    "contractions and script variants are not rewritten from provider output. "
    "Spoken number forms versus digits can therefore create edit differences."
)
SAFE_LABEL = re.compile(r"^[a-zA-Z0-9_.-]{1,128}$")


class AnalysisError(Exception):
    """A fixed diagnostic label, never an underlying exception or source text."""


def read_json(path, required=True):
    try:
        if not path.exists() and not required:
            return {}
        if not path.is_file() or path.stat().st_size > MAX_JSON_BYTES:
            raise AnalysisError("input_invalid_or_too_large")
        with path.open("r", encoding="utf-8") as handle:
            value = json.load(handle)
        if not isinstance(value, dict):
            raise AnalysisError("input_schema_invalid")
        return value
    except (OSError, ValueError, UnicodeError):
        raise AnalysisError("input_read_failed") from None


def read_jsonl(path, required=True):
    if not path.exists() and not required:
        return []
    try:
        if not path.is_file() or path.stat().st_size > MAX_JOURNAL_BYTES:
            raise AnalysisError("journal_invalid_or_too_large")
        rows = []
        with path.open("rb") as handle:
            while line := handle.readline(MAX_LINE_BYTES + 1):
                if len(line) > MAX_LINE_BYTES:
                    raise AnalysisError("journal_line_too_large")
                if line.strip():
                    value = json.loads(line)
                    if not isinstance(value, dict):
                        raise AnalysisError("journal_schema_invalid")
                    rows.append(value)
        return rows
    except (OSError, ValueError, UnicodeError):
        raise AnalysisError("journal_read_failed") from None


def safe_label(value):
    return value if isinstance(value, str) and SAFE_LABEL.fullmatch(value) else "unknown"


def numeric(value):
    return value if isinstance(value, (int, float)) and not isinstance(value, bool) else None


def normalize(text, unit):
    text = unicodedata.normalize("NFKC", text).casefold().replace("’", "'")
    if unit == "word":
        text = re.sub(r"(?<=[^\W\d_])[-\u2010\u2011](?=[^\W\d_])", "", text)
        return re.findall(r"[^\W_]+(?:'[^\W_]+)*", text, flags=re.UNICODE)
    if unit == "char":
        return [c for c in text if unicodedata.category(c)[0] in "LNM"]
    raise AnalysisError("comparison_unit_invalid")


def alignment(reference, hypothesis):
    """Levenshtein path; exact/diagonal, deletion, insertion tie preference.

    Byte backpointers bound memory; a size overflow returns unknown, never a
    silently truncated score. Operation anchors locate insertions between cues.
    """
    n, m = len(reference), len(hypothesis)
    if (n + 1) * (m + 1) > MAX_ALIGNMENT_CELLS:
        return None
    width = m + 1
    directions = bytearray((n + 1) * width)
    previous = array("I", range(width))
    for j in range(1, width):
        directions[j] = 3
    for i, token in enumerate(reference, 1):
        current = array("I", [i])
        directions[i * width] = 2
        for j, other in enumerate(hypothesis, 1):
            diagonal = previous[j - 1] + (token != other)
            deletion = previous[j] + 1
            insertion = current[j - 1] + 1
            cost = min(diagonal, deletion, insertion)
            current.append(cost)
            directions[i * width + j] = (
                1 if cost == diagonal else 2 if cost == deletion else 3
            )
        previous = current
    operations = []
    i, j = n, m
    while i or j:
        direction = directions[i * width + j]
        if direction == 1:
            i -= 1
            j -= 1
            operations.append({
                "kind": "match" if reference[i] == hypothesis[j] else "substitution",
                "referenceIndex": i, "hypothesisIndex": j, "anchor": i,
            })
        elif direction == 2:
            i -= 1
            operations.append({"kind": "deletion", "referenceIndex": i,
                               "hypothesisIndex": None, "anchor": i})
        else:
            j -= 1
            operations.append({"kind": "insertion", "referenceIndex": None,
                               "hypothesisIndex": j, "anchor": i})
    return list(reversed(operations))


def compare(cues, hypothesis_text, unit, available=True, expected_speech=True):
    cue_tokens = [normalize(cue["text"], unit) for cue in cues]
    reference = [token for tokens in cue_tokens for token in tokens]
    hypothesis = normalize(hypothesis_text, unit)
    result = {"unit": unit, "referenceUnits": len(reference),
              "hypothesisUnits": len(hypothesis), "normalization": NORMALIZATION}
    if not expected_speech:
        if reference:
            raise AnalysisError("negative_reference_not_empty")
        return {**result, "status": ("evidence_unavailable" if not available else
                "unexpected_transcription" if hypothesis else "no_transcription_observed"),
                "unexpectedTranscription": bool(hypothesis) if available else None,
                "errorRate": None, "cues": [],
                "limitation": "A negative control has no error-rate denominator. This assesses observed final lexical text, not acoustic ground truth or draft absence."}
    if not available or not reference:
        return {**result, "status": "evidence_unavailable" if not available else "reference_empty",
                "errorRate": None, "cues": []}
    path = alignment(reference, hypothesis)
    if path is None:
        return {**result, "status": "alignment_limit", "errorRate": None, "cues": []}
    totals = Counter(operation["kind"] for operation in path)
    result.update({"status": "compared", "matches": totals["match"],
                   "deletions": totals["deletion"], "insertions": totals["insertion"],
                   "substitutions": totals["substitution"],
                   "errorRate": (totals["deletion"] + totals["insertion"] +
                                 totals["substitution"]) / len(reference)})
    owner = [index for index, tokens in enumerate(cue_tokens) for _ in tokens]
    buckets = [[] for _ in cues]
    for operation in path:
        anchor = min(operation["anchor"], len(owner) - 1)
        buckets[owner[anchor]].append(operation)
    reports = []
    for index, (cue, operations) in enumerate(zip(cues, buckets)):
        counts = Counter(operation["kind"] for operation in operations)
        errors = []
        for operation in operations:
            if operation["kind"] == "match":
                continue
            ri, hi = operation["referenceIndex"], operation["hypothesisIndex"]
            errors.append({"kind": operation["kind"],
                           "reference": reference[ri] if ri is not None else None,
                           "hypothesis": hypothesis[hi] if hi is not None else None})
        status = ("matched" if not errors else "partial" if counts["match"] else "unmatched")
        reports.append({"cueId": cue.get("cue_id", cue.get("sentence_id", index + 1)),
                        "reference": cue["text"], "startSeconds": cue.get("start_seconds"),
                        "endSeconds": cue.get("end_seconds"),
                        "timingBasis": cue.get("timing_basis", "unknown"),
                        "status": status, "matches": counts["match"],
                        "deletions": counts["deletion"], "insertions": counts["insertion"],
                        "substitutions": counts["substitution"], "tokenDifferences": errors})
    result["cues"] = reports
    result["cueCounts"] = dict(Counter(cue["status"] for cue in reports))
    result["alignmentLimitCells"] = MAX_ALIGNMENT_CELLS
    result["cueAlignmentNote"] = (
        "Global edit alignment, not word timing. Insertions at a cue boundary belong "
        "to the following cue; repeated words can have several equally optimal paths."
    )
    return result


def asr_finals(rows):
    finals = {}
    missing_identity = 0
    revisions = 0
    for index, row in enumerate(rows):
        if row.get("kind") != "transcription" or row.get("isFinal") is not True:
            continue
        sentence_id = row.get("sentenceId")
        if not isinstance(sentence_id, (str, int)) or isinstance(sentence_id, bool):
            sentence_id = ("unidentified", index)
            missing_identity += 1
        if sentence_id in finals:
            revisions += 1
        first_index = finals.get(sentence_id, {}).get("firstIndex", index)
        text = row.get("text")
        finals[sentence_id] = {
            "sentenceId": row.get("sentenceId"), "text": text if isinstance(text, str) else "",
            "textEvidenceAvailable": isinstance(text, str),
            "firstIndex": first_index, "eventLine": index + 1,
            "elapsedMs": numeric(row.get("elapsedMs")),
            "beginTimeMs": numeric(row.get("beginTimeMs")),
            "endTimeMs": numeric(row.get("endTimeMs")),
        }
    ordered = sorted(finals.values(), key=lambda item: item["firstIndex"])
    return ordered, {"missingSentenceIds": missing_identity, "finalRevisions": revisions}


def observation_key(observation):
    fields = ("source", "generation", "contentRevision", "transportSequence", "eventKind")
    values = tuple(observation.get(key) for key in fields)
    if any(value is None for value in values):
        return None
    if not all(isinstance(values[index], int) and not isinstance(values[index], bool)
               for index in (1, 2, 3)):
        return None
    return values


def provider_stages(rows, entries):
    decisions = {}
    for entry in entries:
        event = entry.get("event", {})
        observation = event.get("observation", {})
        if event.get("kind") != "provider" or observation.get("producer", "session") != "session":
            continue
        key = observation_key(observation)
        if key is not None:
            decisions.setdefault(key, []).append({"traceEventId": entry.get("id"),
                                                  "admission": observation.get("admission")})
    recognition = {}
    pairs = []
    missing_identity = 0
    rejected = {"staleContent", "staleGeneration", "paused", "obsoleteProducer",
                "producerBackpressure", "oversizedText", "producerClosed"}
    for index, row in enumerate(rows):
        event = row.get("event", {})
        observation = event.get("observation", {})
        content = event.get("content") or {}
        if event.get("kind") != "provider" or observation.get("admission") != "queueReliableAttempt":
            continue
        source = safe_label(observation.get("source"))
        source_text = content.get("source")
        if source_text is None and content.get("role") == "source":
            source_text = content.get("text")
        if observation.get("producer") == "recognition" and observation.get("eventKind") == "sourceFinal":
            owner = observation.get("utteranceId")
            identity = (source, observation.get("generation"), observation.get("contentRevision"), owner)
            if owner is None:
                missing_identity += 1
                identity = (*identity, observation.get("transportSequence", index), index)
            first_index = recognition.get(identity, {}).get("firstIndex", index)
            recognition[identity] = {
                "source": source, "text": source_text if isinstance(source_text, str) else "",
                "textEvidenceAvailable": isinstance(source_text, str),
                "privateEventLine": index + 1, "elapsedMs": numeric(row.get("elapsedMs")),
                "firstIndex": first_index, "utteranceId": owner,
                "generation": observation.get("generation"),
                "contentRevision": observation.get("contentRevision"),
                "transportSequence": observation.get("transportSequence"),
            }
        if observation.get("producer") != "provider" or observation.get("eventKind") not in {"confirmedPair", "finalPair"}:
            continue
        key = observation_key(observation)
        matched = decisions.get(key, []) if key is not None else []
        admissions = {item["admission"] for item in matched}
        if admissions == {"accepted"}:
            status = "accepted"
        elif admissions == {"duplicateFinal"}:
            status = "duplicate"
        elif admissions and admissions <= rejected:
            status = "rejected"
        else:
            status = "unknown"
        pairs.append({"source": source, "text": source_text if isinstance(source_text, str) else "",
                      "textEvidenceAvailable": isinstance(source_text, str),
                      "translation": content.get("translation", ""), "acceptance": status,
                      "sourceUtteranceId": numeric(content.get("sourceUtteranceId")),
                      "pairId": numeric(content.get("pairId")),
                      "privateEventLine": index + 1, "elapsedMs": numeric(row.get("elapsedMs")),
                      "generation": observation.get("generation"),
                      "contentRevision": observation.get("contentRevision"),
                      "transportSequence": observation.get("transportSequence"),
                      "eventKind": observation.get("eventKind"), "matchedTraceEvents": matched})
    raw = sorted(recognition.values(), key=lambda item: item["firstIndex"])
    return raw, pairs, missing_identity


def translation_chains(rows, recognition, pairs):
    """Identity joins only; equal text or worker owner is never a foreign key."""
    requests, receipts = {}, {}
    for index, row in enumerate(rows):
        event = row.get("event", {})
        if event.get("lane") != "final":
            continue
        identity = tuple(event.get(key) for key in (
            "source", "generation", "contentRevision", "sourceUtteranceId", "pairId"))
        if any(value is None for value in identity):
            continue
        if event.get("kind") == "translationRequest":
            requests.setdefault(identity, []).append({**event, "privateEventLine": index + 1})
        elif event.get("kind") == "translationAttemptResult":
            receipts.setdefault((identity, event.get("requestId")), []).append({**event, "privateEventLine": index + 1})
    chains = []
    for pair in pairs:
        identity = tuple(pair.get(key) for key in (
            "source", "generation", "contentRevision", "sourceUtteranceId", "pairId"))
        source_matches = [item for item in recognition if
                          (item["source"], item["generation"], item["contentRevision"], item["utteranceId"]) == identity[:4]]
        attempts = []
        for request in requests.get(identity, []):
            results = receipts.get((identity, request.get("requestId")), [])
            body = request.get("body", {})
            text = body.get("text")
            if isinstance(text, list):
                text = text[0] if len(text) == 1 else None
            if text is None:
                users = [message.get("content") for message in body.get("messages", []) if message.get("role") == "user"]
                text = users[0] if len(users) == 1 else None
            decoded = [result for result in results if result.get("outcome") == "decoded" and not result.get("outputLimited")]
            attempts.append({"requestId": request.get("requestId"), "requestPrivateEventLine": request["privateEventLine"],
                             "requestSourceMatchesRecognition": (text == source_matches[0]["text"]
                                 if len(source_matches) == 1 and source_matches[0]["textEvidenceAvailable"] and isinstance(text, str) else None),
                             "resultPrivateEventLines": [result["privateEventLine"] for result in results],
                             "outcomes": [safe_label(result.get("outcome")) for result in results],
                             "decodedOutputMatchesPair": decoded[0].get("output") == pair["translation"] if len(decoded) == 1 else None})
        chains.append({"source": pair["source"], "generation": pair["generation"], "contentRevision": pair["contentRevision"],
                       "sourceUtteranceId": pair["sourceUtteranceId"], "pairId": pair["pairId"],
                       "pairPrivateEventLine": pair["privateEventLine"], "transportSequence": pair["transportSequence"],
                       "acceptance": pair["acceptance"], "matchedTraceEvents": pair["matchedTraceEvents"],
                       "status": "identity_unavailable" if any(value is None for value in identity) else
                                 "exact_identity_join" if len(source_matches) == 1 and attempts else "identity_join_incomplete",
                       "recognitionPrivateEventLines": [item["privateEventLine"] for item in source_matches],
                       "pairSourceMatchesRecognition": pair["text"] == source_matches[0]["text"] if len(source_matches) == 1 else None,
                       "attempts": attempts,
                       "limitation": "Missing HTTP attempts may include same-language local completion. No source/pair identity is inferred for legacy producers."})
    return chains


def trace_entries(case_directory, report):
    """Prefer bounded private disk tracing; distinguish it from ring eviction."""
    trace = report.get("trace", {})
    entries = trace.get("entries", trace.get("events", []))
    if not isinstance(entries, list):
        raise AnalysisError("trace_schema_invalid")
    path = case_directory / "trace-events.jsonl"
    durable = report.get("tracePersistenceEnabled") is True and path.is_file()
    if durable:
        entries = read_jsonl(path)
    ids = [entry.get("id") for entry in entries]
    if any(not isinstance(value, int) or isinstance(value, bool) or value < 1 for value in ids):
        raise AnalysisError("trace_event_identity_invalid")
    unique = sorted(set(ids))
    recorded = trace.get("recorded")
    if not isinstance(recorded, int) or isinstance(recorded, bool) or recorded < 0:
        recorded = None
    missing = None if recorded is None else max(0, recorded - len(unique))
    invalid_upper_bound = recorded is not None and any(value > recorded for value in unique)
    duplicates = len(ids) - len(unique)
    counters = {key: report.get(key) for key in (
        "tracePersistenceEnabled", "persistedTraceEntries", "traceBytes", "traceDropped", "traceLimited", "traceFailed"
    )}
    disk_loss = durable and (any(counters.get(key) for key in ("traceDropped", "traceLimited", "traceFailed"))
                             or missing != 0 or duplicates > 0 or invalid_upper_bound)
    complete = (missing == 0 and duplicates == 0 and not invalid_upper_bound and
                not disk_loss and (durable or not trace.get("evicted")))
    return sorted(entries, key=lambda entry: entry["id"]), {
        "source": "durable_file" if durable else "memory_ring",
        "observedEntries": len(entries), "missingEventIds": missing,
        "duplicateEventIds": duplicates, "invalidUpperBound": invalid_upper_bound,
        "complete": complete, "memoryEvicted": trace.get("evicted"),
        "onDiskLossOrIncomplete": disk_loss if durable else None, **counters,
    }


def history_rows(snapshots):
    history = {}
    missing_identity = 0
    max_entries = 0
    collisions = set()
    preexisting = set()
    capture_started = False
    for index, row in enumerate(snapshots):
        snapshot = row.get("snapshot", {})
        capture_started = capture_started or snapshot.get("isActive") is True
        subtitles = snapshot.get("subtitles", {})
        tracks = subtitles.get("tracks", snapshot.get("subtitleTracks", []))
        groups = [(None, subtitles.get("history", []))]
        groups.extend((track.get("audioSource"), track.get("history", [])) for track in tracks)
        max_entries = max(max_entries, len(subtitles.get("history", [])),
                          sum(len(track.get("history", [])) for track in tracks))
        snapshot_values = {}
        for track_source, pairs in groups:
            group_seen = set()
            for pair in pairs:
                timestamp = numeric(pair.get("createdAt"))
                source = safe_label(pair.get("audioSource", track_source or "system"))
                if timestamp is None:
                    missing_identity += 1
                    continue
                identity = (source, timestamp)
                value = (pair.get("source", ""), pair.get("translation", ""))
                # Top-level history mirrors tracks. Identical copies across
                # representations are not two utterances; duplicates within one
                # list or conflicting copies have ambiguous durable identity.
                if identity in group_seen or (
                    identity in snapshot_values and snapshot_values[identity] != value
                ):
                    collisions.add(identity)
                group_seen.add(identity)
                snapshot_values[identity] = value
                if not capture_started and snapshot.get("isActive") is False:
                    preexisting.add(identity)
                first_index = history.get(identity, {}).get("firstSnapshotIndex", index)
                history[identity] = {
                    "source": source, "createdAt": timestamp,
                    "text": pair.get("source", ""), "translation": pair.get("translation", ""),
                    "textEvidenceAvailable": isinstance(pair.get("source"), str),
                    "firstSnapshotIndex": first_index, "lastSnapshotLine": index + 1,
                    "snapshotId": snapshot.get("debugSnapshotId"),
                    "elapsedMs": numeric(row.get("elapsedMs")),
                    "sourceDefaulted": "audioSource" not in pair and track_source is None,
                }
    # Dict insertion order preserves real utterance order, including identical text.
    return [item for key, item in history.items() if key not in collisions and key not in preexisting], {
                                    "missingHistoryIdentities": missing_identity,
                                    "ambiguousHistoryIdentities": len(collisions),
                                    "ambiguousHistorySources": sorted({source for source, _ in collisions}),
                                    "preexistingHistoryIdentitiesExcluded": len(preexisting),
                                    "identityCollisionStatus": "ambiguous" if collisions else "none_observed",
                                    "identityLimitation": "Same audioSource/createdAt across snapshots is a revision; collision within one snapshot cannot identify separate utterances and is excluded from scoring. Initial inactive history belongs to the preexisting display baseline, not this clip.",
                                    "maximumSnapshotHistoryEntries": max_entries,
                                    "knownOverlayHistoryCapacity": 20,
                                    "historyCapacityReached": max_entries >= 20}


def frontend_summary(entries):
    counts = Counter()
    overlay = []
    unflushed = set()
    for entry in entries:
        event = entry.get("event", {})
        if event.get("kind") == "stopped":
            unflushed.update(safe_label(value) for value in event.get("unflushedWindows", []))
        if event.get("kind") != "frontend":
            continue
        observation = event.get("observation", {})
        counts[f'{safe_label(observation.get("window"))}/{safe_label(observation.get("stage"))}'] += 1
        if observation.get("window") != "overlay" or observation.get("stage") != "overlayCommitted":
            continue
        keys = ("snapshotId", "projectionRevision", "sourceCharacters", "translationCharacters",
                "historyEntries", "audioTracks", "selectedSourceCharacters",
                "selectedTranslationCharacters", "stableSourceCharacters", "stableTranslationCharacters",
                "visibleCharacters", "visibleBlocks", "overflowedBlocks", "scrollTop", "scrollHeight",
                "viewportHeight", "viewportWidth", "clientDropped")
        overlay.append({"traceEventId": numeric(entry.get("id")),
                        "elapsedMs": numeric(entry.get("elapsedMs")),
                        "projection": safe_label(observation.get("projection")),
                        **{key: numeric(observation.get(key)) for key in keys}})
    return {"textErrorRate": None, "textComparisonStatus": "numeric_observations_only",
            "stages": dict(counts), "overlayObservations": overlay,
            "overflowObservationCount": sum((item["overflowedBlocks"] or 0) > 0 for item in overlay),
            "maximumOverflowedBlocks": max((item["overflowedBlocks"] or 0 for item in overlay), default=0),
            "unflushedWindows": sorted(unflushed),
            "limitation": "Clipping observations are candidate geometry evidence, not missing text or proof of readable native visibility. Batch receipt elapsedMs is not exact browser paint time."}


def baseline_directory(directory, clip_label):
    if (directory / "events.jsonl").is_file():
        return directory
    index_path = directory / "baseline-index.json"
    if index_path.exists():
        matches = [entry for entry in read_json(index_path).get("clips", [])
                   if entry.get("baseLabel", entry.get("label")) == clip_label]
        if len(matches) == 1 and isinstance(matches[0].get("eventsPath"), str):
            return Path(matches[0]["eventsPath"]).parent
    candidates = []
    for child in sorted(directory.iterdir()):
        if child.is_dir() and (child / "metrics.json").is_file():
            label = read_json(child / "metrics.json").get("label")
            if label in {clip_label, clip_label + "-auto"}:
                candidates.append(child)
    if len(candidates) != 1:
        raise AnalysisError("baseline_selection_ambiguous_or_missing")
    return candidates[0]


def language_code(value):
    mapping = {"English": "en", "Chinese": "zh", "Japanese": "ja", "Automatic": "auto"}
    return mapping.get(value, safe_label(value))


def configuration(route, request, metrics, clip, private_rows=None, utterances=None):
    payload = request.get("payload", {})
    parameters = payload.get("parameters", {})
    hints = parameters.get("language_hints")
    baseline_language = (language_code(hints[0]) if isinstance(hints, list) and len(hints) == 1
                         else "auto" if not hints else "unknown")
    actual_model = safe_label(payload.get("model"))
    case_language = language_code(route.get("sourceLanguage"))
    model_inferred = route.get("provider") == "alibabaCloud"
    observed = []
    identities = {(item.get("source"), item.get("generation")) for item in (utterances or [])}
    for index, row in enumerate(private_rows or []):
        event = row.get("event", {})
        if event.get("kind") != "asrRequest" or event.get("protocol") != "audio3":
            continue
        identity = (event.get("source"), event.get("generation"))
        if identities and identity not in identities:
            continue
        body = event.get("body", {}).get("payload", {})
        observed_parameters = body.get("parameters", {})
        observed_hints = observed_parameters.get("language_hints")
        observed_language = (language_code(observed_hints[0]) if isinstance(observed_hints, list) and len(observed_hints) == 1
                             else "auto" if not observed_hints else "unknown")
        limited = event.get("bodyLimited") is True
        observed.append({"source": safe_label(identity[0]), "generation": numeric(identity[1]),
                         "contentRevision": numeric(event.get("contentRevision")), "privateEventLine": index + 1,
                         "model": safe_label(body.get("model")), "language": observed_language,
                         "matchedRecognitionGeneration": identity in identities, "bodyLimited": limited,
                         "sampleRateHz": numeric(observed_parameters.get("sample_rate")),
                         "format": safe_label(observed_parameters.get("format")),
                         "semanticPunctuation": observed_parameters.get("semantic_punctuation_enabled") is True,
                         "heartbeat": observed_parameters.get("heartbeat") is True,
                         "parametersEqualToBaseline": None if limited else observed_parameters == parameters,
                         "contextEqualToBaseline": None if limited else body.get("input", {}) == payload.get("input", {}),
                         "boundary": "websocketTaskRequestPrepared", "serverReceiptProven": False})
    if observed:
        model_match = ("unknown" if any(item["bodyLimited"] or item["model"] == "unknown" for item in observed)
                       else "observed_match" if all(item["model"] == actual_model for item in observed)
                       else "observed_different")
        language_match = ("unknown" if any(item["bodyLimited"] or item["language"] == "unknown" for item in observed)
                          else "match" if all(item["language"] == baseline_language for item in observed) else "different")
    else:
        model_match = (("inferred_match" if actual_model == "qwen-audio-3.0-asr-flash-streaming"
                        else "different") if model_inferred else "unknown")
        language_match = ("unknown" if "unknown" in (case_language, baseline_language)
                          else "match" if case_language == baseline_language else "different")
    return {"baseline": {"model": actual_model, "language": baseline_language,
                         "sampleRateHz": numeric(parameters.get("sample_rate")),
                         "format": safe_label(parameters.get("format")),
                         "semanticPunctuation": parameters.get("semantic_punctuation_enabled") is True,
                         "inputProcessing": safe_label(metrics.get("input_processing")),
                         "fixturePreparation": safe_label(metrics.get("fixture_preparation"))},
            "mimi": {key: safe_label(route.get(key)) for key in
                     ("provider", "sourceLanguage", "targetLanguage", "translationMode", "audioInput")},
            "languageMatch": language_match, "modelMatch": model_match,
            "modelMatchBasis": ("Observed private production-encoded ASR prepared requests; server receipt is not proven."
                                if observed else "Legacy case: Alibaba route default ASR model inferred from repository; actual ASR request unavailable."),
            "observedAsrRequests": observed,
            "referenceLanguage": language_code(clip.get("language")),
            "requestContextRecorded": isinstance(payload.get("input"), dict),
            "limitation": "A direct paced PCM baseline bypasses OS playback/capture. Matching model/language does not prove identical acoustic input, preprocessing, context or provider determinism."}


def audio_summary(case_directory, report, clip):
    result = []
    expected = clip.get("pcm_sha256")
    if expected is None and isinstance(clip.get("pcm_path"), str):
        path = Path(clip["pcm_path"])
        if path.is_file() and path.stat().st_size <= MAX_AUDIO_BYTES:
            expected = hashlib.sha256(path.read_bytes()).hexdigest()
    for source in ("system", "microphone"):
        path = case_directory / "audio-evidence" / (source + ".wav")
        if not path.is_file():
            path = case_directory / ("sent-" + source + ".wav")
        if not path.is_file():
            continue
        if path.stat().st_size > MAX_AUDIO_BYTES:
            raise AnalysisError("audio_evidence_too_large")
        try:
            with wave.open(str(path), "rb") as handle:
                raw = handle.readframes(handle.getnframes())
                checksum = hashlib.sha256(raw).hexdigest()
                result.append({"source": source, "sampleRateHz": handle.getframerate(),
                               "channels": handle.getnchannels(), "sampleWidthBytes": handle.getsampwidth(),
                               "durationSeconds": handle.getnframes() / handle.getframerate(),
                               "pcmSha256": checksum,
                               "referencePcmEquality": "unknown" if expected is None else
                               "equal" if checksum == expected else "different"})
        except (wave.Error, OSError, EOFError):
            raise AnalysisError("audio_evidence_invalid") from None
    status = report.get("audio", {})
    losses = []
    for item in status.get("sources", []):
        losses.append({"source": safe_label(item.get("source")), **{
            key: numeric(item.get(key)) for key in ("attemptedFrames", "successfulChunks", "failedChunks",
                                                  "cancelledChunks", "droppedChunks", "bytes", "sampleRateHz")}})
    return {"boundary": "local_socket_send_completed", "serverReceiptProven": False,
            "recordings": result, "sendCounters": losses,
            "limited": status.get("limited") is True, "failedStorage": status.get("failedStorage") is True,
            "limitation": "WAV concatenates successful sends and may include capture silence. A differing whole-file hash proves nonidentical PCM, not missing speech. Use the audio index and listening to locate gaps."}


def text_of(items):
    return "\n".join(item["text"] for item in items if isinstance(item.get("text"), str))


def negative_absence_evidence(source, manifest, report, coverage, rows):
    """Empty collections alone cannot establish a completed negative control."""
    enabled = (manifest.get("containsPrivateAudioAndSubtitles") is True or
               report.get("route", {}).get("recordedContent") is True)
    sent = any(item.get("source") == source and (numeric(item.get("successfulChunks")) or 0) > 0
               for item in report.get("audio", {}).get("sources", []))
    prepared = any(row.get("event", {}).get("kind") == "asrRequest" and
                   row["event"].get("source") == source and not row["event"].get("bodyLimited")
                   for row in rows)
    return (enabled and sent and prepared and coverage["complete"] and
            report.get("trace", {}).get("enabled") is False and
            not any(report.get(key) for key in ("contentDropped", "contentLimited", "contentFailed")) and
            not any(report.get("audio", {}).get(key) for key in ("limited", "failedStorage")))


def analyze(case_directory, media_manifest, clip_label, asr_directory):
    clips = read_json(media_manifest).get("clips", [])
    matches = [clip for clip in clips if clip.get("label") == clip_label]
    if len(matches) != 1:
        raise AnalysisError("reference_selection_ambiguous_or_missing")
    clip = matches[0]
    unit = clip.get("unit")
    expected_speech = clip.get("expected_speech", clip.get("expectedSpeech", True))
    if type(expected_speech) is not bool:
        raise AnalysisError("speech_expectation_invalid")
    reference_path = clip.get("reference_path")
    if not expected_speech and reference_path is None:
        reference_text = ""
    elif not isinstance(reference_path, str):
        raise AnalysisError("reference_missing")
    else:
        path = Path(reference_path)
        if not path.is_file() or path.stat().st_size > MAX_LINE_BYTES:
            raise AnalysisError("reference_invalid_or_too_large")
        reference_text = path.read_text(encoding="utf-8")
    cues = clip.get("sentences") or [{"sentence_id": index + 1, "text": line,
                                     "timing_basis": "untimed_reference_line"}
                                    for index, line in enumerate(reference_text.splitlines()) if line.strip()]
    if any(not isinstance(cue, dict) or not isinstance(cue.get("text"), str) for cue in cues):
        raise AnalysisError("reference_schema_invalid")
    if normalize(text_of(cues), unit) != normalize(reference_text, unit):
        raise AnalysisError("reference_cues_disagree_with_transcript")
    if not expected_speech and normalize(reference_text, unit):
        raise AnalysisError("negative_reference_not_empty")
    manifest = read_json(case_directory / "manifest.json")
    report = read_json(case_directory / "trace.json")
    trace = report.get("trace", {})
    entries, trace_coverage = trace_entries(case_directory, report)
    rows = read_jsonl(case_directory / "events.jsonl", required=False)
    snapshots = read_jsonl(case_directory / "snapshots.jsonl", required=False)
    asr_directory = baseline_directory(asr_directory, clip_label)
    baseline_rows = read_jsonl(asr_directory / "events.jsonl")
    request = read_json(asr_directory / "request.json", required=False)
    metrics = read_json(asr_directory / "metrics.json", required=False)
    finals, baseline_identity = asr_finals(baseline_rows)
    raw, pairs, raw_missing_identity = provider_stages(rows, entries)
    history, history_meta = history_rows(snapshots)
    route = report.get("route", manifest.get("route", {}))
    evidence_enabled = manifest.get("containsPrivateAudioAndSubtitles") is True or route.get("recordedContent") is True
    private_provider_observed = any(row.get("event", {}).get("kind") == "provider" for row in rows)
    stages = {"directAsrFinal": {"utterances": finals,
                               "comparison": compare(cues, text_of(finals), unit,
                                   all(item["textEvidenceAvailable"] for item in finals) and
                                   (expected_speech or (any(row.get("kind") == "taskFinished" for row in baseline_rows)
                                                       and metrics.get("failure") is None)), expected_speech)}}
    sources = sorted({item["source"] for item in raw + pairs + history})
    if not sources:
        sources = [route.get("audioInput") if route.get("audioInput") in ("system", "microphone") else "unknown"]
    for source in sources:
        absence_available = not expected_speech and negative_absence_evidence(source, manifest, report, trace_coverage, rows)
        for name, items, available in (
            ("recognitionFinal", [item for item in raw if item["source"] == source],
             evidence_enabled and private_provider_observed or absence_available),
            ("acceptedPairSource", [item for item in pairs if item["source"] == source and item["acceptance"] == "accepted"],
             any(item["source"] == source and item["acceptance"] != "unknown" for item in pairs) or absence_available),
            ("durableHistorySource", [item for item in history if item["source"] == source],
             evidence_enabled and bool(snapshots)),
        ):
            stages[name + "/" + safe_label(source)] = {
                "utterances": items, "comparison": compare(cues, text_of(items), unit, available and
                    all(item["textEvidenceAvailable"] for item in items) and
                    not (name == "durableHistorySource" and source in history_meta["ambiguousHistorySources"]), expected_speech),
                "textCoverageComplete": all(item["textEvidenceAvailable"] for item in items),
                "referenceSourceAssignment": "not_proven; each source compared separately without mixing",
            }
    losses = {key: trace.get(key) for key in ("recorded", "evicted", "entryLimit", "frontendDropped", "staleFrontendRejected")}
    losses.update({key: report.get(key) for key in ("privateEvents", "replaySnapshots", "contentBytes",
                                                   "contentDropped", "contentLimited", "contentFailed")})
    frontend = frontend_summary(entries)
    warnings = ["source_reference_not_provider_ground_truth", "cue_alignment_is_not_word_timing",
                "frontend_observations_have_no_text_wer", "direct_pcm_differs_from_playback_capture_path"]
    if trace.get("enabled") is not False:
        warnings.append("case_may_be_unfinished")
    if any(item["acceptance"] == "unknown" for item in pairs):
        warnings.append("pair_acceptance_unknown_without_exact_trace_join")
    if frontend["unflushedWindows"]:
        warnings.append("frontend_stop_flush_incomplete")
    if history_meta["historyCapacityReached"]:
        warnings.append("bounded_history_capacity_reached")
    if history_meta["ambiguousHistoryIdentities"]:
        warnings.append("history_identity_collision_unknown")
    if any(not item.get("textEvidenceAvailable") for item in finals + raw + pairs + history):
        warnings.append("private_text_evidence_missing")
    if not trace_coverage["complete"]:
        warnings.append("trace_coverage_incomplete_or_unknown")
    if any(losses.get(key) for key in ("frontendDropped", "contentDropped", "contentLimited", "contentFailed")) or trace_coverage["onDiskLossOrIncomplete"]:
        warnings.append("evidence_loss_or_limit_present")
    return {"schemaVersion": 1, "normalizationVersion": "nfkc-word-internal-hyphen-v1", "containsPrivateTranscripts": True,
            "clipLabel": clip_label, "caseId": safe_label(report.get("caseId", manifest.get("id"))),
            "buildRevision": safe_label(route.get("buildRevision")),
            "reference": {"language": language_code(clip.get("language")), "unit": unit,
                          "expectedSpeech": expected_speech,
                          "status": safe_label(clip.get("reference_status")),
                          "license": clip.get("license"), "cueCount": len(cues),
                          "limitations": clip.get("baseline_limitations", [])},
            "configuration": configuration(route, request, metrics, clip, rows, raw + pairs),
            "baselineCompletion": {"taskFinishedObserved": any(row.get("kind") == "taskFinished" for row in baseline_rows),
                                   "failurePresent": metrics.get("failure") is not None,
                                   **baseline_identity},
            "stages": stages, "providerPairs": pairs,
            "translationChains": translation_chains(rows, raw, pairs),
            "pairAcceptanceCounts": dict(Counter(item["acceptance"] for item in pairs)),
            "recognitionMissingUtteranceIds": raw_missing_identity,
            "history": history_meta, "frontend": frontend, "evidenceCounters": losses,
            "traceCoverage": trace_coverage,
            "audio": audio_summary(case_directory, report, clip), "warnings": warnings,
            "interpretation": "These are observed stage transcripts, not automatic root-cause proof. A missing cue may arise before ASR, during recognition, translation, backend admission, projection or visibility. Text translation accuracy needs a separate bilingual reference and review."}


def markdown(report):
    lines = ["# Private Mimi dev case analysis", "", "This report contains source and recognized speech. Keep it private.", "",
             "## Stage comparison", "", "| Stage | Status | Reference units | Hypothesis units | Error rate | D / I / S |",
             "|---|---|---:|---:|---:|---:|"]
    for name, stage in report["stages"].items():
        score = stage["comparison"]
        rate = f'{score["errorRate"]:.4f}' if score["errorRate"] is not None else "unknown"
        lines.append(f'| {name} | {score["status"]} | {score["referenceUnits"]} | {score["hypothesisUnits"]} | {rate} | '
                     f'{score.get("deletions", "?")} / {score.get("insertions", "?")} / {score.get("substitutions", "?")} |')
    lines.extend(["", NORMALIZATION, "", "## Per-cue differences", ""])
    for name, stage in report["stages"].items():
        lines.extend([f"### {name}", ""])
        for cue in stage["comparison"]["cues"]:
            timing = (f'{cue["startSeconds"]}–{cue["endSeconds"]} s' if cue["startSeconds"] is not None
                      and cue["endSeconds"] is not None else "time unknown")
            lines.extend([f'Cue {cue["cueId"]}: {cue["status"]}; {timing}.', "",
                          "```text", cue["reference"].replace("```", "'''"), "```", ""])
            if cue["tokenDifferences"]:
                lines.extend(["```json", json.dumps(cue["tokenDifferences"], ensure_ascii=False, indent=2), "```", ""])
    lines.extend(["## Evidence and limits", "", report["interpretation"], "",
                  report["configuration"]["limitation"], "", report["audio"]["limitation"], "",
                  report["frontend"]["limitation"], "", "```json", json.dumps({
                      "configuration": report["configuration"], "pairAcceptanceCounts": report["pairAcceptanceCounts"],
                      "history": report["history"], "evidenceCounters": report["evidenceCounters"],
                      "traceCoverage": report["traceCoverage"],
                      "frontendStages": report["frontend"]["stages"],
                      "overflowObservationCount": report["frontend"]["overflowObservationCount"],
                      "unflushedWindows": report["frontend"]["unflushedWindows"],
                      "warnings": report["warnings"],
                  }, ensure_ascii=False, indent=2), "```", ""])
    return "\n".join(lines)


def write_private(directory, report):
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    if directory.is_symlink() or not directory.is_dir():
        raise AnalysisError("output_directory_invalid")
    if os.name == "posix":
        directory.chmod(0o700)
    for name, content in (("analysis.json", json.dumps(report, ensure_ascii=False, indent=2) + "\n"),
                          ("analysis.md", markdown(report))):
        descriptor = os.open(directory / name, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            handle.write(content)


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise AnalysisError("invalid_arguments")


def main(argv=None):
    parser = Parser(description=__doc__)
    for argument in ("case", "media-manifest", "clip", "asr-result", "output-directory"):
        parser.add_argument("--" + argument, required=True)
    try:
        arguments = parser.parse_args(argv)
        report = analyze(Path(arguments.case), Path(arguments.media_manifest), arguments.clip,
                         Path(arguments.asr_result))
        write_private(Path(arguments.output_directory), report)
        print(json.dumps({"status": "complete", "stages": len(report["stages"]),
                          "referenceCues": report["reference"]["cueCount"],
                          "unknownPairs": report["pairAcceptanceCounts"].get("unknown", 0)}))
        return 0
    except AnalysisError as error:
        print(json.dumps({"status": str(error)}), file=sys.stderr)
        return 2
    except (OSError, ValueError, TypeError, KeyError, AttributeError, UnicodeError):
        print(json.dumps({"status": "analysis_failed"}), file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
