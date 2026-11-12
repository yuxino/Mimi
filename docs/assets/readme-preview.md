# Localized README images

The public README uses a matching image for each language: UI labels and translated subtitles follow the reader’s language. English shows Japanese speech translated into English; other locales show English speech translated into that locale. Do not reuse the Chinese image in English posts.

## Regenerate

```sh
npm run readme:images
npm run readme:images -- --locale en
npm run readme:images -- --check
```

Requires the repository’s supported Node version, installed npm dependencies, and Chrome or Chromium. Set `MIMI_DOCS_BROWSER` to the executable if it is not in a standard location. Generation bundles only the documentation fixture, uses an isolated headless browser over a pipe, and removes its temporary files and browser profile on exit. It starts no server, reads no `.env`, and downloads no browser or dependencies. Font rendering depends on the host; macOS has the fonts used for the committed images. On Linux, install fonts covering CJK, Thai and Cyrillic before rendering.

The command writes `readme-preview-<locale>.webp`, updates only the marked image blocks in the registered READMEs, and records image hashes, UI source revision and whether the UI had local changes in `readme-images.json`. `--check` checks locale coverage, image hashes and Markdown references; it does not prove images still match later UI edits. Rerender after changing the overlay.

## Add a language

1. Ensure Mimi’s UI already supports the language in `src/lib/i18n.ts`.
2. Add an entry to [locales.json](../../scripts/readme-images/locales.json): UI/source/target language codes, three original fictional lines and their translations, localized alt text, and the README paths. Use a different source language from the translation language.
3. Add the localized README with one `<!-- mimi-preview:start -->` / `<!-- mimi-preview:end -->` block where its image belongs. Update the README language navigation as appropriate.
4. Run `npm run readme:images`, inspect the output for glyphs, wrapping and readable subtitles, then run `npm run readme:images -- --check` and `git diff --check`.

The check rejects missing artwork configurations for new UI languages or new localized READMEs. Generation verifies the actual UI language codes, all three subtitle pairs and vertical clipping before saving an image. Long translations may require adjusting the shared fixture dimensions or font size; do not alter product components to fit promotional material.

## Sources and boundaries

- `readme-background.png` is the original imagegen illustration from 2026-10-04 (1672 × 941). It is reused unchanged.
- [stage.html](../../scripts/readme-images/stage.html) composes that background with a 760 × 300 logical-pixel overlay at 1.28 scale, in a 1920 × 1080 frame.
- [overlay.html](../../scripts/readme-images/overlay.html) imports Mimi’s actual `OverlayWindow`, embedded controls, styles and localization. Native calls are stubbed and allowlisted; external requests are blocked. No product component or internal style is modified.
- Captions are fictional dialogue, not a live recognition run. The image does not measure native capture, provider connectivity or latency. No personal audio, transcripts, API credentials or recording session is used. History and audio saving remain off; latency is unavailable rather than invented.
- The retired video, poster and obsolete tour documentation have been removed. Do not re-add the old demo links or their explanatory paragraph to READMEs. A future video should be recorded anew from the current app, with matching UI/caption language and documented recording boundaries.

## Illustration prompt

Use case: illustration-story.
Asset type: one high-resolution 16:9 landscape background illustration for Mimi, a live audio translation and subtitle app. Generate a brand-new refined anime film still. No interface or text: the genuine subtitle window will be placed over the quiet LEFT side afterward.
Scene: just after a light rain outside a small independent cinema, pale plaster facade, a softly glowing recessed entrance far at the right, damp pavement with very restrained reflections. A quiet, intimate, believable moment of reunion, not an advertisement, not a workplace, not a railway station. No readable signs, no posters with writing. Muted warm gray, ivory, charcoal, delicate natural light. No saturated colorful backdrop.
Subject: Mimi, a charming adult anime woman with soft pastel pink shoulder-length twin tails, airy loose strands and a tiny curled ahoge, small floral hair clips, expressive violet eyes. Preserve the existing pink-haired Mimi identity. She wears a beautiful simple ivory blouse with gently puffed sleeves and a charcoal high-waisted flowing skirt; subtle fabric folds, understated natural clothes. No jacket, blazer, waistcoat, headphones, headset, school uniform, techwear or deliberate cool-girl styling. Her expression is a warm slightly surprised smile, glancing toward someone just off-camera to the LEFT. One hand holds a small worn blank cinema ticket near her chest, the other relaxed. No second visible character.
Composition: cinematic medium shot from just below the waist, subject in the RIGHTMOST 40% with her face centered roughly at 79% width and 31% height. Avoid cropping hair or head. Her body turns toward the open left side, creating a natural visual link to the subtitles which will sit at x=3% to 55%, y=38% to 76%. The LEFT 57% should be mostly quiet pale plaster and soft out-of-focus atmosphere, enough empty space for a wide real translation popup; no objects or busy detail there. Full-bleed scene, not a boxed character cutout, no split panels. Keep the scene airy, beautifully composed and tactile, with real depth.
Style: polished Japanese theatrical animation / detailed visual-novel key visual, delicate confident linework, fine hair detail, luminous but understated eyes, sophisticated soft natural light and carefully drawn hands. Beautifully finished, not chibi, not plastic 3D.
Text: none. No logos, text, subtitles, fake UI, screen, decorative arrow, watermark or border. Highest available image quality, landscape 16:9.
