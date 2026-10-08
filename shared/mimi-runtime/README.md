# Mimi shared live runtime

Desktop and Android import the same Rust provider implementations. This crate
owns live provider clients/protocols, immutable configurations and language
capabilities, Audio3 + Qwen-MT/independent translation, speculative drafts and
prioritized finals, bounded PCM/events, the session controller, MT budget
continuity, reconnect schedule and health/pending timing. `mimi-core` owns pure
subtitle reduction and the realtime display projection.

Desktop `clients`/portable `core` modules re-export this crate; Apple native
bindings are behind `apple-native`. Development debugger hooks use the same
state through the matching feature. Android JNI supplies platform proxy/trust
snapshots and capture PCM; Kotlin receives the already reduced controller
snapshot. There is no Kotlin live request scheduler.

The crate's tests use synthetic fixtures and local mock sockets. Provider
account/capture/device acceptance requires separate explicit runtime evidence.
