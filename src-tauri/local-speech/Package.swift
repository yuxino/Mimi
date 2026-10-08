// swift-tools-version:6.2
import PackageDescription

let package = Package(
    name: "MimiLocalSpeech",
    platforms: [.macOS(.v14)],
    products: [.executable(name: "mimi-local-speech", targets: ["MimiLocalSpeech"])],
    dependencies: [
        .package(url: "https://github.com/Blaizzy/mlx-audio-swift.git", revision: "dbe5eaac964e8257785f9d015c81f819a38016a8")
    ],
    targets: [.executableTarget(name: "MimiLocalSpeech", dependencies: [
        .product(name: "MLXAudioCore", package: "mlx-audio-swift"),
        .product(name: "MLXAudioSTT", package: "mlx-audio-swift")
    ])]
)
