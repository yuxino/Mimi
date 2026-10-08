# Settings appearance (Issue #34)

The settings window supports System, Light, and Dark appearances under General.
System is the default and follows live OS appearance changes. An explicit choice
is remembered in WebView local storage, scoped to this settings surface; it does
not change subtitle contrast over video or the tray's existing presentation.
Only the appearance preference is stored, never subtitle or audio content.

Reuse the neutral settings tokens. The sidebar layout and appearance preview
controls follow the [current UI guidelines](../development/ui-guidelines.md). Light mode uses a white
panel on a near-white canvas, dark text, and neutral borders. Every neutral alpha
uses the theme's foreground channels so selection, hover, focus, controls, and
scrollbars remain legible. Selection uses explicit classes for WKWebView.

Storage failures retain the chosen appearance for the current window. System
appearance listeners are removed when the settings surface unmounts. All labels
follow the seven supported interface languages.

Validation: repository checks and the signed macOS development app in UI-only
mode. Inspect category selection, service forms, General controls and dropdowns
in both appearances; verify that reloading retains an explicit choice.
