"""Time an isolated native pause/resume capture without saving screenshots."""

from collections import Counter, deque
from contextlib import contextmanager
from dataclasses import dataclass
from statistics import median
from threading import Event, Thread
from time import monotonic_ns

from PIL import ImageGrab

NS_PER_MS = 1_000_000
FRAME_INTERVAL_NS = 12 * NS_PER_MS
PRE_CLICK_WINDOW_NS = 170 * NS_PER_MS
FINAL_WINDOW_NS = 190 * NS_PER_MS
CORE_SIZE = 16
MIN_STABLE_FRAMES = 4
MAX_MATCH_DISTANCE = 0.16
MIN_GLYPH_CHANGE = 0.32
MIN_ENDPOINT_DISTANCE = 0.40


@dataclass(frozen=True)
class Frame:
    started_ns: int
    ended_ns: int
    luminance: bytes

    def interval(self):
        return [self.started_ns, self.ended_ns]


def glyph_mask(luminance):
    # Exclude the button border and normalize foreground contrast against its
    # fill so hover, disabled opacity and color transitions cannot decide shape.
    background = median(luminance)
    peak = sorted(luminance)[-5]
    threshold = background + max(18, (peak - background) * 0.28)
    return frozenset(index for index, value in enumerate(luminance) if value >= threshold)


def distance(left, right):
    union = left | right
    return len(left ^ right) / len(union) if union else 0.0


def stable_mask(masks, label):
    if len(masks) < MIN_STABLE_FRAMES:
        raise RuntimeError(f"Too few {label} pause-button pixel samples")
    occurrences = Counter(pixel for mask in masks for pixel in mask)
    reference = frozenset(
        pixel for pixel, count in occurrences.items() if count > len(masks) // 2
    )
    if not 10 <= len(reference) <= 90:
        raise RuntimeError(f"The {label} pause-button glyph did not have a usable silhouette")
    if any(
        min(pixel % CORE_SIZE, pixel // CORE_SIZE) == 0
        or max(pixel % CORE_SIZE, pixel // CORE_SIZE) == CORE_SIZE - 1
        for pixel in reference
    ):
        raise RuntimeError(f"The {label} pause-button glyph touched the edge of the capture")
    deviation = max(distance(mask, reference) for mask in masks)
    if deviation > MAX_MATCH_DISTANCE:
        raise RuntimeError(f"The {label} pause-button glyph did not stabilize")
    return reference, deviation


def meaningful_pixel_change(luminance, reference):
    # A visible fill or icon change must move multiple pixels by at least ten
    # luma levels. Single-pixel noise is not a button feedback observation.
    return sum(abs(value - expected) >= 10 for value, expected in zip(luminance, reference)) >= 8


def first_bracket(frames, first_index, matches, label):
    for index in range(first_index, len(frames)):
        if matches(index):
            if matches(index - 1):
                raise RuntimeError(f"No preceding {label} nonmatch was captured")
            previous, current = frames[index - 1:index + 1]
            return index, {
                "previous_capture_ns": previous.interval(),
                "matching_capture_ns": current.interval(),
            }
    raise RuntimeError(f"No {label} was captured")


class ButtonPixelRecorder:
    def __init__(self, point):
        x, y = map(int, point)
        radius = CORE_SIZE // 2
        self.crop = (x - radius, y - radius, x + radius, y + radius)
        self.frames = deque(maxlen=512)
        self.stop = Event()
        self.error = None

    def capture(self):
        try:
            while not self.stop.is_set():
                started_ns = monotonic_ns()
                # ImageGrab extracts the X11 composited desktop. Retain only the
                # luminance of the tiny local button crop and never write it.
                image = ImageGrab.grab(bbox=self.crop)
                ended_ns = monotonic_ns()
                self.frames.append(Frame(started_ns, ended_ns, image.convert("L").tobytes()))
                remaining_ns = FRAME_INTERVAL_NS - (monotonic_ns() - started_ns)
                if remaining_ns > 0:
                    self.stop.wait(remaining_ns / 1_000_000_000)
        except Exception as error:
            self.error = error

    def summarize(self, clicked_ns, enabled_ns):
        if self.error is not None:
            raise RuntimeError("Pause-button screenshot sampling failed") from self.error
        if not self.stop.is_set():
            raise RuntimeError("Finish pause-button pixel sampling before summarizing it")
        return summarize_frames(tuple(self.frames), clicked_ns, enabled_ns)


def summarize_frames(frames, clicked_ns, enabled_ns):
    if not frames or not frames[0].ended_ns < clicked_ns <= enabled_ns <= frames[-1].ended_ns:
        raise RuntimeError("Pause-button screenshots did not bracket the click and final enable")
    masks = tuple(glyph_mask(frame.luminance) for frame in frames)
    first_after = next(index for index, frame in enumerate(frames) if frame.ended_ns >= clicked_ns)
    before_indices = [
        index for index in range(first_after)
        if frames[index].started_ns >= clicked_ns - PRE_CLICK_WINDOW_NS
    ]
    after_start = max(enabled_ns, frames[-1].ended_ns - FINAL_WINDOW_NS)
    after_indices = [
        index for index in range(first_after, len(frames))
        if frames[index].started_ns >= after_start
    ]
    before, before_deviation = stable_mask([masks[index] for index in before_indices], "pre-click")
    after, after_deviation = stable_mask([masks[index] for index in after_indices], "final")
    endpoint_distance = distance(before, after)
    if endpoint_distance < MIN_ENDPOINT_DISTANCE:
        raise RuntimeError("Pause-button glyphs before and after the click were too similar")

    baseline_luminance = tuple(
        median(values) for values in zip(*(frames[index].luminance for index in before_indices))
    )
    if any(meaningful_pixel_change(frames[index].luminance, baseline_luminance) for index in before_indices):
        raise RuntimeError("Pause-button pixels changed appreciably before the click")
    _, first_pixels = first_bracket(
        frames, first_after,
        lambda index: meaningful_pixel_change(frames[index].luminance, baseline_luminance),
        "meaningful pixel change",
    )
    _, first_glyph_change = first_bracket(
        frames, first_after, lambda index: distance(masks[index], before) >= MIN_GLYPH_CHANGE,
        "glyph shape change",
    )

    def matches_final(index):
        return (
            distance(masks[index], after) <= MAX_MATCH_DISTANCE
            and distance(masks[index], before) >= MIN_GLYPH_CHANGE
        )

    final_index, first_final = first_bracket(frames, first_after, matches_final, "final glyph match")
    if not all(matches_final(index) for index in range(final_index, len(frames))):
        raise RuntimeError("The final pause-button glyph match reverted during sampling")

    measured = frames[before_indices[0]:]
    capture_costs = [frame.ended_ns - frame.started_ns for frame in measured]
    capture_intervals = [
        current.started_ns - previous.started_ns for previous, current in zip(measured, measured[1:])
    ]
    return {
        "clicked_ns": clicked_ns,
        "enabled_ns": enabled_ns,
        "first_meaningful_pixel_change": first_pixels,
        "first_glyph_change": first_glyph_change,
        "first_final_glyph_match": first_final,
        "sampling": {
            "count": len(measured),
            "capture_cost_median_ns": int(median(capture_costs)),
            "capture_cost_max_ns": max(capture_costs),
            "capture_interval_median_ns": int(median(capture_intervals)),
            "capture_interval_max_ns": max(capture_intervals),
        },
        "confidence": {
            "before_samples": len(before_indices),
            "after_samples": len(after_indices),
            "before_glyph_pixels": len(before),
            "after_glyph_pixels": len(after),
            "before_mask_max_distance": before_deviation,
            "after_mask_max_distance": after_deviation,
            "opposite_mask_distance": endpoint_distance,
        },
    }


@contextmanager
def record_button_pixels(point):
    recorder = ButtonPixelRecorder(point)
    sampler = Thread(target=recorder.capture, name="pause-button-pixels", daemon=True)
    sampler.start()
    try:
        yield recorder
    finally:
        recorder.stop.set()
        sampler.join(timeout=3)
        if sampler.is_alive():
            recorder.error = RuntimeError("Pause-button screenshot sampler did not stop")
