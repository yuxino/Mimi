"""Native overlay geometry and XRender visual checks for the isolated X11 probe."""

from contextlib import suppress
import re
import subprocess
from typing import NamedTuple


class Bounds(NamedTuple):
    x: int
    y: int
    width: int
    height: int


def window_fields(command, window):
    fields = {}
    for line in command("xwininfo", "-id", window).splitlines():
        key, separator, value = line.partition(":")
        if separator:
            fields[key.strip()] = value.strip()
    return fields


def bounds(fields):
    return Bounds(*(int(fields[key]) for key in (
        "Absolute upper-left X", "Absolute upper-left Y", "Width", "Height",
    )))


def require_bounds(command, wait_for, overlay, expected):
    latest = None

    def current():
        nonlocal latest
        fields = window_fields(command, overlay)
        latest = bounds(fields)
        if latest == expected and fields["Map State"] == "IsViewable":
            return latest
        return None

    try:
        return wait_for(f"the mapped overlay at {expected}", current, seconds=5)
    except RuntimeError as error:
        raise RuntimeError(f"Overlay did not reach {expected}. Last bounds were {latest}.") from error


def resize_corner(command, overlay, current, dx, dy):
    # Match linux-ci-smoke.sh: the 14px grip is inside the client area.
    command(
        "xdotool", "mousemove", "--window", overlay,
        str(current.width - 10), str(current.height - 10),
        "mousedown", "1", "sleep", "0.10", "mousemove_relative", "--",
        str(dx), str(dy), "sleep", "0.10", "mouseup", "1",
    )
    return current._replace(width=current.width + dx, height=current.height + dy)


def visual_identity(fields):
    return fields["Visual"], fields["Visual Class"], int(fields["Depth"])


def render_visual(command, identity):
    visual, visual_class, depth = identity
    output = command("xdpyinfo", "-ext", "RENDER")
    mapping = re.search(
        rf"(?m)^[ \t]*visual id:[ \t]+{re.escape(visual)}[ \t]*\n"
        r"[ \t]*pict format id:[ \t]+(0x[0-9a-f]+)",
        output,
    )
    if mapping is None:
        raise RuntimeError(f"No XRender picture format was reported for visual {visual}.")
    format_id = mapping[1]
    direct_format = re.search(
        rf"(?m)^[ \t]*format id:[ \t]+{re.escape(format_id)}[ \t]*\n"
        r"[ \t]*type:[ \t]+Direct[ \t]*\n"
        r"[ \t]*depth:[ \t]+(\d+)[ \t]*\n"
        r"[ \t]*alpha:[ \t]+(\d+)[ \t]+mask[ \t]+(0x[0-9a-f]+)",
        output,
    )
    if direct_format is None or int(direct_format[1]) != depth:
        raise RuntimeError(f"XRender format {format_id} did not describe visual {visual} at depth {depth}.")
    return {
        "id": visual,
        "class": visual_class,
        "depth": depth,
        "render_format": format_id,
        "alpha_shift": int(direct_format[2]),
        "alpha_mask": int(direct_format[3], 16),
    }


def verify_overlay_geometry(overlay, settings, command, wait_for):
    """Verify a native corner round-trip and report visual format, without sampling alpha pixels."""
    overlay, settings = str(overlay), str(settings)
    fields = window_fields(command, overlay)
    if fields["Map State"] != "IsViewable":
        raise RuntimeError("The subtitle overlay was unmapped before the corner drag.")
    initial = bounds(fields)
    identity = visual_identity(fields)
    visual = render_visual(command, identity)
    actions = command("xprop", "-id", overlay, "_NET_WM_ALLOWED_ACTIONS")
    if not actions.startswith("_NET_WM_ALLOWED_ACTIONS(ATOM) = ") or "_NET_WM_ACTION_RESIZE" in actions:
        raise RuntimeError(f"GTK advertised unexpected window-manager actions for the overlay. {actions.strip()}")

    pointer = dict(line.split("=", 1) for line in command("xdotool", "getmouselocation", "--shell").splitlines())
    # Leave space for the corner drag even when the default overlay hugs a screen edge.
    test_bounds = initial._replace(x=120, y=140)
    try:
        if test_bounds != initial:
            command("wmctrl", "-ir", overlay, "-e", "0,120,140,-1,-1")
            require_bounds(command, wait_for, overlay, test_bounds)

        enlarged = resize_corner(command, overlay, test_bounds, 32, 24)
        require_bounds(command, wait_for, overlay, enlarged)
        if visual_identity(window_fields(command, overlay)) != identity:
            raise RuntimeError("The native X11 visual changed during enlargement.")

        restored = resize_corner(command, overlay, enlarged, -32, -24)
        require_bounds(command, wait_for, overlay, restored)
        if restored != initial:
            command("wmctrl", "-ir", overlay, "-e", f"0,{initial.x},{initial.y},-1,-1")
            restored = require_bounds(command, wait_for, overlay, initial)
        if visual_identity(window_fields(command, overlay)) != identity:
            raise RuntimeError("The native X11 visual changed after restoring the overlay.")
    except Exception:
        # A timed-out xdotool chain can leave its down event outstanding.
        with suppress(subprocess.SubprocessError):
            command("xdotool", "mouseup", "1")
            command("xdotool", "mousemove", pointer["X"], pointer["Y"])
        raise
    command("xdotool", "mousemove", pointer["X"], pointer["Y"])

    if window_fields(command, settings)["Map State"] != "IsViewable" or "WM_DELETE_WINDOW" not in command(
        "xprop", "-id", settings, "WM_PROTOCOLS",
    ):
        raise RuntimeError("Settings was no longer available for the normal window close request.")

    return {"initial": initial._asdict(), "enlarged": enlarged._asdict(),
            "restored": restored._asdict(), "visual": visual}
