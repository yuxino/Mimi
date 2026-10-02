# Provider brand assets

Retrieved from official sites on 2026-10-02 and 2026-10-03. Assets are bundled locally in
`src/assets/providers`; service settings never fetch icons at runtime.
`ProviderIcon` uses a 32 px image inside a 32 or 36 px container and preserves
each image's aspect ratio and brand colors. Marks identify the selected service;
their ownership remains with the respective providers.

| Provider | Local asset | Official source | Preparation |
| --- | --- | --- | --- |
| Alibaba Cloud | `alibaba-cloud.svg` | [Official homepage](https://www.alibabacloud.com/en) header SVG | Extracted the original symbol's rect and two paths; omitted the wordmark, preserved geometry and orange fill. |
| OpenAI | `openai-black.svg`, `openai-white.svg` | [Brand guidelines](https://openai.com/brand/), [official logo ZIP](https://cdn.openai.com/brand/OpenAI-Logos-2025.zip) | Unmodified monoblossom SVGs; choose the provided white version in dark settings. |
| Gemini | `gemini.svg` | [Official Gemini site](https://gemini.google.com/), [official SVG](https://www.gstatic.com/lamda/images/gemini_sparkle_aurora_33f86dc0c0257da337c63.svg) | Unmodified SVG, including its embedded raster artwork. |
| Azure | `azure.svg` | [Microsoft shared media index](https://learn.microsoft.com/static/media/index.html), [official SVG](https://learn.microsoft.com/static/media/product/azure/azure.svg) | Unmodified Azure brand symbol; avoids the general Microsoft favicon. |
| Volcano Engine | `volcano-engine.png` | [Official homepage](https://www.volcengine.com/), [official icon](https://portal.volccdn.com/obj/volcfe/misc/favicon.png) | Original transparent 192 px PNG. |
| Tencent Cloud | `tencent-cloud.svg` | [Official homepage](https://cloud.tencent.com/), [official navigation SVG](https://cloudcache.tencent-cloud.com/open_proj/proj_qcloud_v2/gateway/portal/css/img/nav/slice/logo-9225-color.svg) | Extracted the original three-color symbol group, omitted the wordmark; paths and colors unchanged. |
| Baidu Translate | `baidu-translate.jpg` | [Official download page](https://fanyi.baidu.com/download?appchannel=wisebottom) links to [Baidu's App Store listing](https://apps.apple.com/cn/app/id605670941); [Apple metadata](https://itunes.apple.com/lookup?id=605670941&country=cn), [official artwork](https://is1-ssl.mzstatic.com/image/thumb/Purple221/v4/a3/39/44/a339447f-a0d6-6090-15af-e277f67511a4/AppIcon-0-0-1x_U007emarketing-0-8-0-sRGB-85-220.png/512x512bb.jpg) | Unmodified 512 × 512 marketing artwork. The metadata identifies `com.baidu.translate`, published by Beijing Baidu Netcom Science & Technology Co.,Ltd. Replaces the 32 px favicon; no local upscaling. |
| xAI | `xai.png` | [Official API console](https://console.x.ai/), [official favicon](https://console.x.ai/favicon.ico) | Lossless ICO to PNG conversion, original 32 px pixels. Uses the API console's xAI mark. |
| DeepL | `deepl-blue.svg`, `deepl-white.svg` | [Official press brand assets](https://www.deepl.com/en/press), [official logo ZIP](https://assets.ctfassets.net/pplyeawnfzc7/2vv9U6WXbhq7TteXDBMIAf/6da6c10c83676960468a3a8b58956757/DeepL_Official_Logo_-20250707T082253Z-1-001.zip) | Extracted the original symbol path from the digital RGB blue and white SVGs; omitted the wordmark, preserved path geometry and original fills. Choose the supplied white version in dark settings. |
| DeepLX adapter | `deeplx.svg` | [Legacy official documentation site](https://deeplx.owo.network/), [published header logo](https://deeplx.owo.network/logo.svg) | Unmodified SVG as published by the legacy documentation. A white backing in dark settings keeps the original black artwork visible without recoloring it. |

The DeepLX asset identifies Mimi's existing legacy adapter using the mark
published by its documentation site. It repeats DeepL's symbol; this does not
claim an independently designed mark or affiliation with DeepL. The
[current upstream repository](https://github.com/OwO-Network/DLX) was renamed
to DLX and removed DeepL branding; this asset does not identify that newer brand.

OpenAI-compatible text translation uses the neutral Lucide `Languages` glyph.
Compatibility identifies an API format, not OpenAI ownership or affiliation.

Recognition and text translation call sites resolve their service separately.
Legacy DeepLX profiles still use Alibaba Cloud's icon for speech recognition;
their text translation uses the legacy DeepLX documentation mark.

Theme selection uses inherited `--provider-light-display`,
`--provider-dark-display`, and `--provider-backing-background` properties.
Portalled service menus copy these properties from their trigger so their
artwork keeps the settings window's selected theme outside its DOM ancestor.
