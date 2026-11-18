# Official credential shortcuts

Built-in speech configuration lacked the official credential action already
available in compatible text presets. Dedicated DeepL and xAI text routes had
the same gap. Keep setup destinations in `shared/provider-copy.json`, generating
the desktop and Android catalogs together.

Show the existing localized Get API key action beside the matching credential
field. Keep Tencent's account/SecretID/SecretKey guidance. Speech products use
their own consoles: Doubao Speech rather than Ark, Baidu Machine Translation
rather than Qianfan, and Tencent CAM rather than TokenHub. Azure opens its
resource console because keys belong to the selected resource.

Navigation is an explicit user action using a catalog ID and page enum, never an
editable endpoint. Opening help must preserve unsaved fields and must not save,
select a profile, check a connection, start capture, or access stored secrets.
Unsupported/custom endpoints do not acquire an assumed official key issuer.
Opening failures use the existing sanitized transient feedback.

Validate catalog generation, native destination restrictions, visible actions
and draft retention in both speech and dedicated text editors. Inspect the
signed macOS fixture at ordinary/minimum widths and in all interface languages.
Android host checks do not establish native or physical-device acceptance.
