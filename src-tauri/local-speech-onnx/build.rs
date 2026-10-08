pub fn build() {
    for path in [
        "local-speech-onnx/build.py",
        "local-speech-onnx/build-test.py",
        "local-speech-onnx/build.rs",
        "local-speech-onnx/runtime-assets.json",
        "local-speech-onnx/main.cpp",
        "local-speech-onnx/worker-test.cpp",
        "local-speech-onnx/CMakeLists.txt",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    let target = std::env::var("TARGET").expect("Rust target");
    assert!(
        std::process::Command::new(if cfg!(windows) { "python" } else { "python3" })
            .args(["-B", "local-speech-onnx/build-test.py"])
            .status()
            .expect("verify ONNX dependency integrity regressions")
            .success(),
        "ONNX build-time dependency integrity regressions failed"
    );
    assert!(std::process::Command::new(if cfg!(windows) { "python" } else { "python3" })
        .args(["local-speech-onnx/build.py", &target])
        .status().expect("prepare pinned native ONNX worker").success(),
        "Native ONNX worker requires Python 3 and CMake at build time, and a C++17 compiler; runtime users need none of these");
}
