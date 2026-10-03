# AppImage GLES runtime dependency

Issue #101 reproduced an official v1.5.6 AppImage startup failure on a minimal
Debian host without `libGLESv2.so.2`. Adding only the distribution's GLES entry
library allowed the existing package to open. This does not establish that
all supported desktops fail, and it is separate from the fixed AppRun
permissions issue.

WebKit loads GLES dynamically, outside linuxdeploy's normal linked-library
scan. The Linux packaging script explicitly adds Ubuntu's `libgles2` GLVND
entry library and copyright notice through Tauri's AppImage file mapping
before bundling and updater signing. The build checks its package source,
architecture, SONAME and expected GLVND dependency. It does not distribute
EGL, GLdispatch or vendor GPU drivers: these stay with the host's graphics
stack, following the [AppImage exclusion rules](https://github.com/AppImage/pkg2appimage/blob/master/excludelist).

The final compressed artifact is extracted and inspected before installation
tests. The gate rejects a missing or invalid GLES entry library, links outside
the bundle, missing copyright notice, and included host graphics libraries.
This catches the original omission even when the build machine already has
GLES installed. Unit fixtures check the artifact gate; the native package job
uses real ELF metadata and launches both installed package formats.

This decision preserves the Ubuntu 22.04 x86_64 release baseline. Xvfb and
extract-and-run tests do not establish physical GPU, direct FUSE, native
Wayland or every-distribution compatibility. Those remain in #66 and #92.
