#!/usr/bin/env python3
"""Verify committed input shapes from the isolated GTK Wayland fixture.

This validates native wl_surface requests, not compositor stacking, global
positioning, GNOME/KDE behavior, or pointer delivery to another application.
"""

import re
import sys
from pathlib import Path

PHASES = {
    "locked-before-map": True,
    "unlocked": False,
    "relocked": True,
    "hidden-unlock-remap": False,
    "unrealize-relock-realize": True,
    "unchanged-lock-remap": True,
    "unchanged-lock-unrealize": True,
    "recreated-locked": True,
    "recreated-unlocked": False,
}
OBJECT = r"[@#](\d+)"


def verify_trace(trace):
    regions = {}
    pending = {}
    input_shapes = {}
    attached = {}
    mapped = {}
    committed = {phase: [] for phase in PHASES}
    requests = {phase: 0 for phase in PHASES}
    seen = []
    ended = []
    phase = None
    for line in trace.splitlines():
        end = re.search(r"MIMI_INPUT_PHASE_END ([a-z-]+)", line)
        if end:
            if end.group(1) != phase:
                raise ValueError("mismatched fixture phase end")
            ended.append(phase)
            phase = None
            continue
        marker = re.search(r"MIMI_INPUT_PHASE ([a-z-]+)", line)
        if marker:
            if phase is not None:
                raise ValueError("fixture phase did not end")
            phase = marker.group(1)
            seen.append(phase)
            continue
        surface_created = re.search(r"\.create_surface\(new id wl_surface" + OBJECT, line)
        if surface_created:
            surface = surface_created.group(1)
            input_shapes[surface] = None
            mapped[surface] = False
            pending.pop(surface, None)
            attached.pop(surface, None)
        create = re.search(r"\.create_region\(new id wl_region" + OBJECT, line)
        if create:
            regions[create.group(1)] = []
        add = re.search(r"wl_region" + OBJECT + r"\.add\((-?\d+), (-?\d+), (\d+), (\d+)\)", line)
        if add:
            region, *coordinates = add.groups()
            if region not in regions:
                raise ValueError("input region was not created in this trace")
            regions[region].append(tuple(map(int, coordinates)))
        if re.search(r"wl_region" + OBJECT + r"\.subtract\(", line):
            raise ValueError("unexpected region subtraction; do not guess its shape")
        shape = re.search(r"wl_surface" + OBJECT + r"\.set_input_region\((nil|null|wl_region[@#]\d+)\)", line)
        if shape:
            surface, region = shape.groups()
            if region in ("nil", "null"):
                pending[surface] = None
            else:
                region_id = re.search(r"[@#](\d+)$", region).group(1)
                if region_id not in regions:
                    raise ValueError("unknown input region")
                pending[surface] = list(regions[region_id])
        attach = re.search(r"wl_surface" + OBJECT + r"\.attach\((nil|null|wl_buffer[@#]\d+),", line)
        if attach:
            surface, buffer = attach.groups()
            attached[surface] = buffer not in ("nil", "null")
        commit = re.search(r"wl_surface" + OBJECT + r"\.commit\(\)", line)
        if commit:
            surface = commit.group(1)
            if surface in pending:
                input_shapes[surface] = pending.pop(surface)
                if phase in requests:
                    requests[phase] += 1
            if surface in attached:
                mapped[surface] = attached.pop(surface)
            # Initial xdg-shell configure commits have no buffer and cannot
            # intercept clicks. Check every effective shape once mapped,
            # never overwrite a bad first frame with a later correct shape.
            if phase in committed and mapped.get(surface, False):
                committed[phase].append(input_shapes.get(surface))
    if seen != [*PHASES, "complete"] or ended != list(PHASES):
        raise ValueError(f"incomplete or out-of-order native fixture: {seen}")
    for phase, locked in PHASES.items():
        shapes = committed[phase]
        if not requests[phase] or not shapes:
            raise ValueError(f"{phase}: no input-region request followed by a mapped wl_surface.commit")
        for shape in shapes:
            if locked and shape != []:
                raise ValueError(f"{phase}: locked input region is not completely empty: {shape}")
            if not locked and shape is not None and not any(
                x <= 10 < x + width and y <= 10 < y + height
                for x, y, width, height in shape
            ):
                raise ValueError(f"{phase}: GTK default input was not restored: {shape}")
    return list(PHASES)


def main():
    if len(sys.argv) != 2:
        raise SystemExit("usage: check-linux-input-region-trace.py <WAYLAND_DEBUG trace>")
    try:
        phases = verify_trace(Path(sys.argv[1]).read_text())
    except ValueError as error:
        raise SystemExit(str(error)) from error
    print(f"Native Wayland input-region protocol passed: {len(phases)} lifecycle phases")
    print("Scope: GTK input-region requests only; not GNOME/KDE positioning, stacking, or full desktop acceptance")


if __name__ == "__main__":
    main()
