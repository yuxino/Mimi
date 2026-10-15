# Research probe notices

`ActivityGate` in `mt-vad-benchmark.py` is a Python adaptation of the
`SileroVadModel::Impl::IsSpeech` algorithm from
[sherpa-onnx v1.13.8](https://github.com/k2-fsa/sherpa-onnx/blob/v1.13.8/sherpa-onnx/csrc/silero-vad-model.cc).
Copyright (c) 2023 Xiaomi Corporation. Licensed under the Apache License 2.0;
the full license is in `licenses/sherpa-onnx.txt`.
Changes: Python implementation, fixed 16 kHz/512-sample clock, numeric-only
results, and matching low-level host comparison. The adapted algorithm remains
under Apache-2.0. Other original research code follows Mimi's repository license.

The external Silero model is not included in Git or in an application bundle.
Its source is [snakers4/silero-vad](https://github.com/snakers4/silero-vad),
revision `1e261b036686cd0017d500ee96acd1c4ba572a9d`,
`src/silero_vad/data/silero_vad_16k_op15.onnx`, under
[MIT](https://github.com/snakers4/silero-vad/blob/1e261b036686cd0017d500ee96acd1c4ba572a9d/LICENSE).
The probe rejects a different SHA256. Actual bundling must preserve that license
and review the runtime's licenses and native artifacts on every shipped platform.
