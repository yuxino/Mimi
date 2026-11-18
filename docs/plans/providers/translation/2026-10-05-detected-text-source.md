# Detected source languages for DeepL and DeepLX

An automatic recognition session can report French or German even though the
text adapter currently offers only Chinese, English, Japanese and Korean as
explicit source hints. Desktop previously promoted that report to an explicit
hint and rejected it locally before sending an HTTP request.

Keep an automatic session automatic when its detected source has no local wire
mapping: omit `source_lang` for official DeepL and send `source_lang: "auto"` to
the DeepLX-compatible service. Retain encodable detected hints and keep explicit
user source choices authoritative, including their existing validation. This
preserves service-side detection; it does not promise that every deployed model
supports the detected language or expand the manual language catalog.

Android already applies this rule in `translationSourceLanguage`; the same
`detectedSourceRequests` fixtures now exercise both platforms' actual request
encoders. Desktop loopback tests also verify the HTTP clients use these encoders.
This is synthetic protocol evidence, not live provider or Android device acceptance.

References: [DeepL automatic source detection](https://developers.deepl.com/api-reference/translate/request-translation)
and [DLX source parameter handling](https://github.com/OwO-Network/DLX/blob/main/translate/translate.go).
