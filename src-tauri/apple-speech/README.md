# Apple Speech platform adapter

This static Swift library exposes language readiness, explicit asset
preparation, and independent 16 kHz mono PCM16LE sessions. It never captures
audio. Normal session start checks the exact module's installed asset state
without reserving or downloading assets. The Rust wrapper owns operation
deadlines, bounded result delivery, and callback routing by unique identifier.

Apple silicon builds require Xcode 26+ with a macOS 26 SDK. The Cargo build
script compiles the bridge for macOS 13 and gates SpeechAnalyzer at runtime on
macOS 26. Intel macOS and other platforms compile an unavailable Rust adapter.
End users do not need Xcode or an external service. Each selected audio source
owns a separate native session.

Only the explicit Download and use action may request system assets. Choosing
a resource language alone does not save or download it. A successful download
applies the selection; a background download remains pending until the user
refreshes its state and applies the ready language. Runtime language
choices are intersected with the selected text translator's implemented source
catalog. Original-only recognition retains the system's full supported set.

Resource inventory distinguishes installed, downloading (including the system
waiting to retry), and missing resources. Apple's installation request reserves
locales automatically and consolidates duplicate downloads. A transient native
service disconnect retries at most once with a fresh request. Installation
completion is checked against the actual module; global installed locales are
not treated as proof. A confirmed install updates only that locale in the latest
cache, after older inventory scans finish, without requiring another full scan.
Errors expose fixed categories only; unknown errors retain an allowlisted domain
and numeric code for content-free diagnostics, never NSError descriptions.

API references: [downloadAndInstall](https://developer.apple.com/documentation/speech/assetinstallationrequest/downloadandinstall()),
[installation request](https://developer.apple.com/documentation/speech/assetinventory/assetinstallationrequest(supporting:)),
and [downloading status](https://developer.apple.com/documentation/speech/assetinventory/status/downloading).

The model-free `tests/PCMQueueTests.swift` source covers bounded overflow, EOF
drain, and cancellation. Rust regression sources cover owned byte copies, late
callbacks, dropped handles, result queue overflow, and draining final results
before completion. CI selects the required Xcode and compiles the product bridge
through Cargo. Model experiment runners and benchmark reports are outside this
product change.

Regression source and compilation are separate from native app acceptance.
The signed Mimi identity, selected-source capture, resource readiness, text
translation, and visible overlay require an explicitly scheduled app check.
