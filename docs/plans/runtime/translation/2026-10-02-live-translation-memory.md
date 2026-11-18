# Keep live requests independent of translation examples

## Evidence and decision

The slow native replay showed a partial source beside a translation resembling
the preceding full-document result. The frontend's Alibaba atomic projection
reads both lanes from the same completed preview pair, and its empty-value
stabilizer clears immediately. Qwen's SSE decoder is created per HTTP request;
neither Qwen nor HQ owns a response cache.

The [official Qwen translation guide](https://help.aliyun.com/zh/model-studio/machine-translation)
defines `tm_list` as source/translation examples whose style and sentence forms
the model imitates for the current translation. It explicitly describes a
single-turn service without multi-turn dialogue. The [API reference](https://help.aliyun.com/zh/model-studio/qwen-mt-api)
separately defines the one user message as the text to translate and the
optional `tm_list` examples. These documents do not define `tm_list` as a
conversation-history channel or guarantee fuzzy cache behavior.

HQ had automatically retained up to twelve confirmed subtitle pairs and sent
two recent pairs with every preview and six with each final. Clearing the
display did not clear those examples. Replaying a previous document could
therefore send its complete old translation beside a new shorter cumulative
prefix. That is a plausible source of output contamination, not a proven
explanation of the observed upstream response.

Stop passing automatically collected session history as translation examples
in both live preview and final requests. Remove that extra retained text from
HQ. The protocol encoder's support for explicitly supplied translation memory
stays intact, as do prompts, terms, model, proxy routing, pacing, bounded retry
and durable FIFO behavior. No hidden fallback or new provider request is added.

## Verification and remaining boundaries

- A real localhost Qwen SSE fixture completes an earlier full final, a new
  shorter prefix preview, then a repeated full final. Every actual request
  contains exactly its current input, no `tm_list`, and independent complete
  responses produce their corresponding atomic pairs.
- Existing protocol fixtures continue to cover explicit `tm_list` encoding.
- Content-free request logs add `sourceLength` and `memoryEntries=0`; complete
  preview/final logs add `sourceLength` and `translationLength`. Values count
  Unicode characters, not words or tokens, and reveal no subtitle text.
- A successful response can still be semantically wrong. Lengths help locate
  anomalous request/response pairs but do not prove translation correctness.
- Manual Clear currently clears the reducer/archive rather than invalidating
  provider work. Preventing an already-pending event from refilling the display
  is a separate lifecycle fix, not established by removing `tm_list`.
- Native slow replay must be repeated on the rebuilt signed app. The earlier
  screenshot is not a passing acceptance result for this source change.

Focused verification passed 43 HQ tests and 42 Qwen client/protocol tests;
the existing credential-free public-network smoke remains intentionally
ignored. Formatting and diff whitespace checks passed. These fixtures prove
the emitted request contract and local state behavior, not the actual model's
semantic correctness on the replay.
