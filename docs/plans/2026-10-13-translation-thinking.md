# Non-thinking live text translation

Live subtitles prioritize response latency. Known text services with optional
thinking receive an explicit non-thinking request. There is no user-facing
thinking switch, new preference, credential migration, or model substitution.

Keep this policy in the shared Rust Chat Completions request builder, so both
desktop sessions/probes and Android JNI sessions/probes inherit it. Match the
validated hostname and documented model capability; do not send provider fields
to arbitrary gateways just because a model name resembles an upstream model.
Never retry a rejected request with thinking enabled.

| Official service | Request field | Scope |
| --- | --- | --- |
| DeepSeek | `thinking.type = disabled` | Existing official-host behavior |
| Alibaba | `enable_thinking = false` | Documented hybrid Qwen/DeepSeek models, including regional and workspace endpoints |
| SiliconFlow | `enable_thinking = false` | Reviewed hybrid Qwen3 and DeepSeek V3.1/V3.2 IDs |
| Kimi | `thinking.type = disabled` | Kimi K2.5 |
| OpenAI | `reasoning_effort = none` | Reviewed GPT families supporting `none`; exclude Pro and mandatory-thinking models |
| Google Chat Completions | `reasoning_effort = none` | Gemini 2.5 Flash/Flash-Lite; exclude Pro and Gemini 3 |
| OpenRouter | `reasoning.enabled = false` | Reviewed optional-thinking model IDs |

Unknown hosts/models and mandatory-thinking variants preserve their existing
requests. This avoids incompatible fields; it does not establish that those
services run without thinking. Alias support is explicit; snapshots inherit
only a documented family, with known mandatory-thinking exceptions excluded.
Add a fixture and verify upstream capabilities before extending this list.

Built-in Qwen-MT has no thinking mode. Grok Voice already sends
`session.reasoning.effort = none`. Dedicated recognition/translation protocols,
including Gemini Live Translate, receive no speculative text-API parameters.

Verification uses shared synthetic endpoint fixtures, direct Rust request
tests, and actual JNI text probes captured by a loopback CONNECT/TLS server.
No live service or credentials are needed for those tests. A separate authorized
latency comparison uses the same short inputs/model/connection, alternates
baseline and non-thinking requests, and reports timings without content or keys.
Native caption latency and translation quality remain separate from HTTP timing.

## 2026-11-13 model maintenance

GPT-6.1 Terra was missing from the reviewed optional-thinking families. Its
[official model specification](https://developers.openai.com/api/docs/models/gpt-6.1-terra)
supports `none` and defaults to `medium`. Add it to the shared request policy
for OpenAI and its existing [OpenRouter route](https://openrouter.ai/openai/gpt-6.1-terra),
with the same host and model-variant boundaries. GPT-6.1 Sol remains excluded:
its [documented efforts](https://developers.openai.com/api/docs/models/gpt-6.1-sol)
do not include `none`.

Shared request fixtures cover the new route and reject an unknown variant and
gateway. Direct Rust and actual JNI request tests consume those same fixtures.
This applies the existing non-thinking policy to a user-selected Terra model; it does not
change the saved model, the default preset, or establish a translation-quality
improvement over Luna.

Protocol references (reviewed 2026-10-13):

- [DeepSeek](https://api-docs.deepseek.com/guides/thinking_mode/)
- [Alibaba](https://help.aliyun.com/zh/model-studio/deep-thinking/)
- [SiliconFlow](https://docs.siliconflow.cn/docs/api/chat-completions-post)
- [Kimi K2.5](https://github.com/MoonshotAI/Kimi-K2.5)
- [OpenAI model capability](https://developers.openai.com/api/docs/guides/latest-model)
- [Google OpenAI compatibility](https://ai.google.dev/gemini-api/docs/openai)
- [OpenRouter](https://openrouter.ai/docs/guides/best-practices/reasoning-tokens)
