#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
target="${1:?target triple required}"
mkdir -p ../binaries
if [[ "$target" != aarch64-apple-darwin ]]; then
  # Intel Macs keep the existing providers; MLX requires Apple Silicon.
  printf '#!/bin/sh\nexit 1\n' > "../binaries/mimi-local-speech-$target"
  chmod +x "../binaries/mimi-local-speech-$target"
  : > ../binaries/mlx.metallib
  mkdir -p ../binaries/swift-transformers_Hub.bundle ../binaries/swift-crypto_Crypto.bundle
  : > ../binaries/swift-transformers_Hub.bundle/unsupported
  : > ../binaries/swift-crypto_Crypto.bundle/unsupported
  exit 0
fi
swift package --disable-automatic-resolution resolve
# SwiftPM's CLI accessor looks at the .app root, where macOS code signing
# rejects resources. Keep resources in Contents/Resources and make the pinned
# Hub fallback honor that standard location (CLI builds retain .module).
hub=.build/checkouts/swift-transformers/Sources/Hub/Hub.swift
if ! grep -Fq 'let mimiResourceBundle' "$hub"; then
  chmod u+w "$hub"
  patch -p1 < hub-resources.patch
fi
swift build -c release --product mimi-local-speech --disable-automatic-resolution
cmake -S .build/checkouts/mlx-swift/Source/Cmlx/mlx -B .build/metal \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_DEPLOYMENT_TARGET=14.0 \
  -DMLX_BUILD_TESTS=OFF -DMLX_BUILD_EXAMPLES=OFF -DMLX_BUILD_BENCHMARKS=OFF \
  -DMLX_BUILD_PYTHON_BINDINGS=OFF -DMLX_METAL_JIT=ON -DMLX_BUILD_GGUF=OFF
cmake --build .build/metal --target mlx-metallib -j4
cmp -s .build/release/mimi-local-speech "../binaries/mimi-local-speech-$target" || cp .build/release/mimi-local-speech "../binaries/mimi-local-speech-$target"
cmp -s .build/metal/mlx/backend/metal/kernels/mlx.metallib ../binaries/mlx.metallib || cp .build/metal/mlx/backend/metal/kernels/mlx.metallib ../binaries/mlx.metallib
# Stage SwiftPM resources for the normal application resource directory.
for bundle in .build/release/*.bundle; do
  [[ -d "$bundle" ]] || continue
  destination="../binaries/$(basename "$bundle")"
  mkdir -p "$destination"
  chmod -R u+w "$destination"
  for resource in "$bundle"/*; do
    cmp -s "$resource" "$destination/$(basename "$resource")" || cp "$resource" "$destination/"
  done
  chmod -R u+w "$destination"
done
