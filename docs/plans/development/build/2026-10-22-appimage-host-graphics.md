# AppImage graphics libraries come from the host

Supersedes `2026-10-03-linux-appimage-gles.md` (kept in Git history).

Issue #101 showed a v1.5.6 AppImage failing to start on a minimal Debian host
without `libGLESv2.so.2`. The earlier fix copied Ubuntu's GLVND entry library
and its copyright notice into the AppImage through Tauri's `appimage.files`
mapping.

That fix made the AppImage own part of the graphics stack. The bundled GLVND
stub shadows the host's `libGLESv2.so.2` through AppRun's library path and is
built for the Ubuntu 22.04 baseline, while `libGLdispatch.so.0` and the actual
EGL/GL drivers remain host-provided. Mixing a packaged dispatch entry with a
different host GLVND stack is the version skew the AppImage excludelist avoids.
The entry library is small, but its presence still makes the AppImage's
graphics behavior depend on the build distribution.

The AppImage now follows the excludelist for the complete graphics stack:
libGL, libEGL, libGLX, libGLdispatch, libOpenGL, libGLESv1_CM, libGLESv2,
libglapi, libgbm, libdrm, vendor drivers and DRI modules must all come from the
host.

Consequences: a host without `libgles2`/GLVND cannot start the AppImage; WebKit
fails before Mimi can render an in-app message. The Linux guide documents the
`libgles2` install. The `.deb` packaging is unchanged.

`scripts/linux/build-linux-packages.sh` no longer injects the library. The final
compressed AppImage is extracted and checked by `scripts/linux/check-appimage-gles.py`,
which rejects any bundled graphics library or the old `libgles2` copyright
notice. `scripts/linux/verify-linux-bundles.sh` runs the check before the install and
X11 smoke tests.

This does not establish physical GPU, direct FUSE, native Wayland or
every-distribution compatibility; those remain in #66 and #92.
