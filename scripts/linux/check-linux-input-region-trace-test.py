#!/usr/bin/env python3
"""Parser regressions; synthetic traces here are never native acceptance."""

import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "trace_check", Path(__file__).with_name("check-linux-input-region-trace.py")
)
trace_check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(trace_check)


def fixture(*, one_pixel=False, commit=True, unlocked_empty=False, version_marker="@"):
    lines = [" -> wl_compositor@4.create_surface(new id wl_surface@8)"]
    for phase, locked in trace_check.PHASES.items():
        lines.append(f"MIMI_INPUT_PHASE {phase}")
        # Reusing object IDs must start a fresh region, not inherit old adds.
        lines.append(" -> wl_compositor@4.create_region(new id wl_region@9)")
        if locked and one_pixel:
            lines.append(" -> wl_region@9.add(0, 0, 1, 1)")
        region = "wl_region@9" if locked or unlocked_empty else "nil"
        lines.append(f" -> wl_surface@8.set_input_region({region})")
        lines.append(" -> wl_region@9.destroy()")
        if commit:
            lines.append(" -> wl_surface@8.attach(wl_buffer@10, 0, 0)")
            lines.append(" -> wl_surface@8.commit()")
        lines.append(f"MIMI_INPUT_PHASE_END {phase}")
    lines.append("MIMI_INPUT_PHASE complete")
    return "\n".join(lines).replace("@", version_marker)


class InputRegionTraceTests(unittest.TestCase):
    def test_empty_and_default_commits_pass(self):
        self.assertEqual(trace_check.verify_trace(fixture()), list(trace_check.PHASES))

    def test_newer_wayland_object_id_format_passes(self):
        trace_check.verify_trace(fixture(version_marker="#"))

    def test_old_tao_one_pixel_region_fails(self):
        with self.assertRaisesRegex(ValueError, "not completely empty"):
            trace_check.verify_trace(fixture(one_pixel=True))

    def test_uncommitted_requests_fail(self):
        with self.assertRaisesRegex(ValueError, "no input-region request"):
            trace_check.verify_trace(fixture(commit=False))

    def test_unlock_must_restore_input(self):
        with self.assertRaisesRegex(ValueError, "default input was not restored"):
            trace_check.verify_trace(fixture(unlocked_empty=True))

    def test_missing_lifecycle_phase_fails(self):
        with self.assertRaisesRegex(ValueError, "incomplete or out-of-order|mismatched fixture"):
            trace_check.verify_trace(fixture().replace("MIMI_INPUT_PHASE recreated-locked", "ignored"))

    def test_region_id_reuse_clears_old_geometry(self):
        trace = " -> wl_compositor@4.create_region(new id wl_region@9)\n -> wl_region@9.add(0, 0, 100, 80)\n"
        trace_check.verify_trace(trace + fixture())

    def test_unlock_can_restore_nonempty_gtk_default_shape(self):
        trace = fixture().replace(
            " -> wl_surface@8.set_input_region(nil)",
            " -> wl_region@9.add(0, 0, 100, 80)\n -> wl_surface@8.set_input_region(wl_region@9)",
        )
        trace_check.verify_trace(trace)

    def test_bad_first_mapped_commit_cannot_be_hidden_by_a_good_commit(self):
        bad = " -> wl_surface@8.set_input_region(nil)\n -> wl_surface@8.attach(wl_buffer@10, 0, 0)\n -> wl_surface@8.commit()\n"
        trace = fixture().replace("MIMI_INPUT_PHASE locked-before-map\n", "MIMI_INPUT_PHASE locked-before-map\n" + bad)
        with self.assertRaisesRegex(ValueError, "not completely empty"):
            trace_check.verify_trace(trace)

    def test_unmapped_configure_handshake_is_not_pointer_delivery(self):
        handshake = " -> wl_surface@8.set_input_region(nil)\n -> wl_surface@8.commit()\n"
        trace = fixture().replace("MIMI_INPUT_PHASE locked-before-map\n", "MIMI_INPUT_PHASE locked-before-map\n" + handshake)
        trace_check.verify_trace(trace)

    def test_unknown_input_region_fails(self):
        with self.assertRaisesRegex(ValueError, "unknown input region"):
            trace_check.verify_trace(fixture().replace("set_input_region(wl_region@9)", "set_input_region(wl_region@99)", 1))


if __name__ == "__main__":
    unittest.main()
