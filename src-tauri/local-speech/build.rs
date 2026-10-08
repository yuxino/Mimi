use std::process::Command;

pub fn build() {
    for path in [
        "local-speech/build.rs",
        "local-speech/build.sh",
        "local-speech/hub-resources.patch",
        "local-speech/Package.swift",
        "local-speech/Package.resolved",
        "local-speech/Sources",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    let target = std::env::var("TARGET").expect("Rust target");
    if !target.ends_with("apple-darwin") {
        return;
    }
    assert!(Command::new("bash").args(["local-speech/build.sh", &target]).status()
        .expect("build local speech helper").success(),
        "Local speech requires Swift 6.3+, CMake, and Xcode Metal Toolchain (xcodebuild -downloadComponent MetalToolchain)");
}
