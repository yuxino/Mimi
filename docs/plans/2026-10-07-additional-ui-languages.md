# Traditional Chinese, Korean, French and German desktop interface languages

## Scope and priority

Add `zh-TW`, `de`, `ko` and `fr` alongside Simplified Chinese, English and Japanese to desktop
interface preferences. Keep native names in the picker. German has the
strongest user-population signal among these three in GitHub's 2025 global
report; Korea and France also appear in public contribution rankings. This
is a reach heuristic, not evidence of Mimi's users' preferred languages.
Mimi's current public issues do not establish a ranking for these locales.
Meow country and browser-language reports are the preferred project-specific
input when available; site visits do not establish installed app usage.

Source: [GitHub Octoverse 2025](https://github.blog/news-insights/octoverse/octoverse-a-new-developer-joins-github-every-second-as-ai-leads-typescript-to-1/).

## Implementation

- Explicit complete primary and supplementary dictionaries, checked against
  existing copy types; no English object spread to mask missing translations.
- Reuse the current in-place locale subscription and backend settings event.
  Do not reload windows or reset ongoing edits, downloads or session state.
- Traditional Chinese is explicitly requested. Resolve `zh-TW`, `zh-HK`,
  `zh-MO` and `zh-Hant` to it; explicit `zh-Hans` retains Simplified Chinese.
- Resolve regional system tags (`de-DE`, `fr-CA`, `ko-KR`) to the matching UI
  language; unsupported languages retain the existing English fallback.
- Use the webview's locale data for expanded language names, preserving wire
  codes, Simplified/Traditional Chinese, Tagalog and original-only semantics.
  Unavailable names retain the existing English name rather than a raw code.
- Localize native menus and Apple's preparation panel too; accept all seven
  interface languages at both native preparation boundaries.
- Korean UI font fallbacks prioritize Korean families. Existing design,
  providers, prompts and capture behavior stay unchanged.
- Release notes select an available matching section, otherwise English.
  Public release authoring remains English followed by Simplified Chinese.

This changes desktop UI only. Android has platform-native UI and is outside
this task; shared subtitle rules and provider contracts are untouched.

## Verification

Run locale key/function completeness, regional detection, in-place changes,
expanded display-name and release-note tests, then `scripts/check.sh`.
Inspect the canonical signed `mimi-dev.app --ui-only` across settings, tray,
control panel and subtitle windows, including narrow widths and long labels.
UI-only checks do not establish capture, service quality or device acceptance.
