#!/usr/bin/env python3
"""Render numeric research evidence, never a simulated application screenshot."""
import argparse
import json
from pathlib import Path


def intervals(scene, host):
    start = None
    result = []
    for event in scene["hosts"][host]["transitions"]:
        at = min(event["detected_ms"], scene["duration_ms"]) / 1000
        if event["active"]:
            start = at
        elif start is not None:
            result.append((start, at - start, False))
            start = None
    if start is not None:
        result.append((start, scene["duration_ms"] / 1000 - start, True))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output already exists")
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    reports = {ms: json.loads((args.results / f"vad-{ms}ms.json").read_text())
               for ms in [250, 400, 600]}
    plt.rcParams.update({"font.size": 10, "axes.spines.top": False,
                         "axes.spines.right": False})
    fig, axes = plt.subplots(2, 1, figsize=(10, 7), gridspec_kw={"height_ratios": [1, 1]})
    fig.suptitle("Silero VAD: same model, two hosts", x=0.11, ha="left", fontsize=18)
    fig.text(0.11, 0.91, "Measured offline on macOS arm64 | 1.23 MiB model | 32 ms frames", color="0.35")
    primary = reports[400]
    names = ["Japanese", "English", "Music / effects", "Digital silence"]
    for index, (host, color) in enumerate([("onnxruntime", "0.12"), ("sherpa-onnx", "0.65")]):
        values = [scene["hosts"][host]["p99_ms"] for scene in primary["scenes"]]
        axes[0].bar([i + (index - 0.5) * 0.32 for i in range(4)], values,
                    width=0.30, color=color, label=host)
    axes[0].set_xticks(range(4), names)
    axes[0].set_ylabel("P99 host call (ms)")
    axes[0].set_ylim(0, 0.20)
    axes[0].legend(frameon=False, loc="upper right")
    axes[0].set_title("3,324 input frames: identical activity decisions", loc="left", pad=12)
    for row, ms in enumerate([250, 400, 600]):
        scene = next(x for x in reports[ms]["scenes"] if x["label"] == "fleurs-english-1527")
        for start, width, at_eof in intervals(scene, "onnxruntime"):
            axes[1].broken_barh([(start, width)], (row - 0.24, 0.48),
                               facecolors="0.70" if at_eof else "0.20",
                               edgecolors="0.20", hatch="///" if at_eof else None)
    axes[1].set_yticks([0, 1, 2], ["250 ms", "400 ms", "600 ms"])
    axes[1].set_xlim(0, 24)
    axes[1].set_xlabel("Input audio time (s)")
    axes[1].set_ylabel("Silence hold")
    axes[1].set_title("English: pause settings change segmentation", loc="left", pad=12)
    axes[1].invert_yaxis()
    fig.text(0.11, 0.06, "Hatched span: still active at EOF; no invented final boundary.\n"
             "Not MT quality, native subtitle latency, or packaged-app performance.", color="0.35", fontsize=9)
    fig.subplots_adjust(left=0.11, right=0.97, top=0.82, bottom=0.17, hspace=0.72)
    fig.savefig(args.output, dpi=200, facecolor="white")
    plt.close(fig)


if __name__ == "__main__":
    main()
