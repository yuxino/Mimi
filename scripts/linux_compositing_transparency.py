"""Check actual X11 desktop pixels around and inside the subtitle canvas."""

from contextlib import contextmanager
from ctypes import CDLL, POINTER, Structure, byref, c_char, c_char_p, c_int, c_long
from ctypes import c_uint, c_ulong, c_ushort, c_void_p

from PIL import ImageGrab

from linux_compositing_interactions import bounds, window_fields


OVERRIDE_REDIRECT = 1 << 9
STACK_WITH_SIBLING = (1 << 5) | (1 << 6)
BELOW = 1
COLORS = ((48, 120, 224), (224, 72, 40))
PIXEL_TOLERANCE = 3


class WindowAttributes(Structure):
    _fields_ = [
        ("background_pixmap", c_ulong), ("background_pixel", c_ulong),
        ("border_pixmap", c_ulong), ("border_pixel", c_ulong),
        ("bit_gravity", c_int), ("win_gravity", c_int), ("backing_store", c_int),
        ("backing_planes", c_ulong), ("backing_pixel", c_ulong), ("save_under", c_int),
        ("event_mask", c_long), ("do_not_propagate_mask", c_long),
        ("override_redirect", c_int), ("colormap", c_ulong), ("cursor", c_ulong),
    ]


class WindowChanges(Structure):
    _fields_ = [
        ("x", c_int), ("y", c_int), ("width", c_int), ("height", c_int),
        ("border_width", c_int), ("sibling", c_ulong), ("stack_mode", c_int),
    ]


class Color(Structure):
    _fields_ = [("pixel", c_ulong), ("red", c_ushort), ("green", c_ushort),
                ("blue", c_ushort), ("flags", c_char), ("pad", c_char)]


@contextmanager
def x11_display():
    x11 = CDLL("libX11.so.6")
    window_pointer = POINTER(c_ulong)
    for name, result, arguments in (
        ("XOpenDisplay", c_void_p, (c_char_p,)),
        ("XCloseDisplay", c_int, (c_void_p,)),
        ("XDefaultScreen", c_int, (c_void_p,)),
        ("XDefaultRootWindow", c_ulong, (c_void_p,)),
        ("XDefaultColormap", c_ulong, (c_void_p, c_int)),
        ("XInternAtom", c_ulong, (c_void_p, c_char_p, c_int)),
        ("XGetSelectionOwner", c_ulong, (c_void_p, c_ulong)),
        ("XQueryTree", c_int, (c_void_p, c_ulong, window_pointer, window_pointer,
                               POINTER(window_pointer), POINTER(c_uint))),
        ("XFree", c_int, (c_void_p,)),
        ("XAllocColor", c_int, (c_void_p, c_ulong, POINTER(Color))),
        ("XCreateSimpleWindow", c_ulong, (c_void_p, c_ulong, c_int, c_int, c_uint, c_uint,
                                         c_uint, c_ulong, c_ulong)),
        ("XChangeWindowAttributes", c_int, (c_void_p, c_ulong, c_ulong, POINTER(WindowAttributes))),
        ("XMapWindow", c_int, (c_void_p, c_ulong)),
        ("XConfigureWindow", c_int, (c_void_p, c_ulong, c_uint, POINTER(WindowChanges))),
        ("XSetWindowBackground", c_int, (c_void_p, c_ulong, c_ulong)),
        ("XClearWindow", c_int, (c_void_p, c_ulong)),
        ("XSync", c_int, (c_void_p, c_int)),
        ("XDestroyWindow", c_int, (c_void_p, c_ulong)),
    ):
        function = getattr(x11, name)
        function.restype = result
        function.argtypes = arguments
    display = x11.XOpenDisplay(None)
    if not display:
        raise RuntimeError("Could not open the isolated X11 display.")
    try:
        yield x11, display
    finally:
        x11.XCloseDisplay(display)


def wait_for_compositor(wait_for):
    with x11_display() as (x11, display):
        if x11.XDefaultScreen(display) != 0:
            raise RuntimeError("The X11 alpha probe requires Xvfb screen zero.")

        def owner():
            selection = x11.XInternAtom(display, b"_NET_WM_CM_S0", True)
            if not selection:
                return None
            return x11.XGetSelectionOwner(display, selection) or None

        return wait_for("the X11 compositor selection owner", owner, seconds=5)


def root_child(x11, display, root, window):
    # Openbox can reparent the app client. Only root siblings may share a stack request.
    for _ in range(16):
        reported_root, parent = c_ulong(), c_ulong()
        children, count = POINTER(c_ulong)(), c_uint()
        okay = x11.XQueryTree(display, window, byref(reported_root), byref(parent), byref(children), byref(count))
        if children:
            x11.XFree(children)
        if not okay or reported_root.value != root or not parent.value:
            break
        if parent.value == root:
            return window
        window = parent.value
    raise RuntimeError("Could not find the overlay frame below the X11 root window.")


def color_pixel(x11, display, colormap, rgb):
    color = Color(red=rgb[0] * 257, green=rgb[1] * 257, blue=rgb[2] * 257)
    if not x11.XAllocColor(display, colormap, byref(color)):
        raise RuntimeError(f"The X11 backplate could not allocate RGB {rgb}.")
    return color.pixel


@contextmanager
def backplate(overlay, geometry):
    with x11_display() as (x11, display):
        root = x11.XDefaultRootWindow(display)
        sibling = root_child(x11, display, root, overlay)
        colormap = x11.XDefaultColormap(display, x11.XDefaultScreen(display))
        # The inherited Xvfb root visual is opaque. A borderless unmanaged
        # window can be placed directly below the app without stealing focus.
        x, y, width, height = geometry
        window = x11.XCreateSimpleWindow(display, root, x - 20, y - 20, width + 40, height + 40,
                                         0, 0, color_pixel(x11, display, colormap, COLORS[0]))
        if not window:
            raise RuntimeError("Could not create the opaque X11 backplate.")
        try:
            attributes = WindowAttributes(override_redirect=True)
            x11.XChangeWindowAttributes(display, window, OVERRIDE_REDIRECT, byref(attributes))
            x11.XMapWindow(display, window)
            changes = WindowChanges(sibling=sibling, stack_mode=BELOW)
            x11.XConfigureWindow(display, window, STACK_WITH_SIBLING, byref(changes))

            def paint(rgb):
                x11.XSetWindowBackground(display, window, color_pixel(x11, display, colormap, rgb))
                x11.XClearWindow(display, window)
                x11.XSync(display, False)

            yield paint
        finally:
            x11.XDestroyWindow(display, window)
            x11.XSync(display, False)


def capture_sample(rgb, points, wait_for):
    # The fixture's 75%-opaque canvas is black. It should pass through 25%
    # of each channel at the blank interior, and all channels in the 6px inset.
    expected = {"outside": rgb, "inset": rgb, "canvas": tuple(round(channel / 4) for channel in rgb)}
    latest = None

    def matches():
        nonlocal latest
        with ImageGrab.grab() as image:
            with image.convert("RGB") as pixels:
                latest = {name: pixels.getpixel(point) for name, point in points.items()}
        for name, target in expected.items():
            if any(abs(actual - channel) > PIXEL_TOLERANCE for actual, channel in zip(latest[name], target)):
                return None
        return latest

    try:
        sample = wait_for(f"composited backdrop RGB {rgb}", matches, seconds=5)
    except RuntimeError as error:
        raise RuntimeError(f"Unexpected composited pixels for backdrop {rgb}. Observed {latest}. Expected {expected}.") from error
    return {"backplate": rgb, **sample}


def window_bounds(command, overlay, settings):
    frames = []
    for window in (overlay, settings):
        fields = window_fields(command, window)
        if fields["Map State"] != "IsViewable":
            raise RuntimeError(f"X11 window {window} was unmapped during the alpha check.")
        frames.append(bounds(fields))
    return frames


def verify_blended_pixels(overlay, settings, command, wait_for):
    """Measure two known backplate colors through the same mapped native overlay."""
    overlay, settings = str(overlay), str(settings)
    owner = wait_for_compositor(wait_for)
    frames = window_bounds(command, overlay, settings)
    x, y, width, height = frames[0]
    with ImageGrab.grab() as image:
        screen_width, screen_height = image.size
    if x < 0 or y < 0 or x + width > screen_width or y + height > screen_height:
        raise RuntimeError("The overlay must fit on the isolated X11 screen for alpha sampling.")

    sample_y = y + height // 2
    points = {"inset": (x + 2, sample_y), "canvas": (x + 26, sample_y)}
    outside_x = x - 12 if x >= 12 else x + width + 12
    if outside_x >= screen_width:
        raise RuntimeError("There was no room for the exposed X11 backplate sample.")
    points["outside"] = (outside_x, sample_y)

    samples = []
    with backplate(int(overlay, 0), frames[0]) as paint:
        for rgb in COLORS:
            paint(rgb)
            samples.append(capture_sample(rgb, points, wait_for))

    if window_bounds(command, overlay, settings) != frames:
        raise RuntimeError("The overlay or Settings frame changed during the alpha check.")
    if "WM_DELETE_WINDOW" not in command("xprop", "-id", settings, "WM_PROTOCOLS"):
        raise RuntimeError("Settings no longer supported the normal window close request.")
    return {"compositor_owner": owner, "points": points, "samples": samples}
