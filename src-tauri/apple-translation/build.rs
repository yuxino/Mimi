use std::path::PathBuf;
use std::process::Command;

fn xcrun(args: &[&str]) -> String {
    let output = Command::new("xcrun")
        .args(args)
        .output()
        .expect("Apple Translation requires Xcode 26 or newer");
    assert!(
        output.status.success(),
        "Apple Translation requires the macOS 26 SDK"
    );
    String::from_utf8(output.stdout)
        .expect("Xcode paths must be UTF-8")
        .trim()
        .to_owned()
}

pub fn build() {
    println!("cargo:rerun-if-changed=apple-translation/Bridge.swift");
    println!("cargo:rerun-if-changed=apple-translation/build.rs");
    println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");
    println!("cargo:rerun-if-env-changed=SDKROOT");
    if std::env::var("TARGET").as_deref() != Ok("aarch64-apple-darwin") {
        return;
    }
    let sdk = PathBuf::from(xcrun(&["--sdk", "macosx", "--show-sdk-path"]));
    let swiftc = PathBuf::from(xcrun(&["--find", "swiftc"]));
    let output = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo output directory"));
    let library = output.join("libMimiAppleTranslation.a");
    let status = Command::new(&swiftc)
        .args([
            "-parse-as-library",
            "-swift-version",
            "6",
            "-warnings-as-errors",
            "-target",
            "arm64-apple-macosx13.0",
            "-sdk",
        ])
        .arg(&sdk)
        .arg("-module-cache-path")
        .arg(output.join("swift-module-cache"))
        .args([
            "-O",
            "-emit-library",
            "-static",
            "-module-name",
            "MimiAppleTranslation",
            "apple-translation/Bridge.swift",
            "-o",
        ])
        .arg(&library)
        .status()
        .expect("run Swift compiler for Apple Translation");
    assert!(
        status.success(),
        "Apple Translation bridge failed to compile; use Xcode 26+ and fix diagnostics above"
    );
    println!("cargo:rustc-link-search=native={}", output.display());
    println!("cargo:rustc-link-lib=static=MimiAppleTranslation");
    // Swift autolink records carry weak framework imports for our macOS 13
    // deployment target. Do not force a strong Translation framework link.
    let swift_lib = swiftc
        .parent()
        .and_then(|p| p.parent())
        .expect("Swift toolchain path")
        .join("lib/swift/macosx");
    println!("cargo:rustc-link-search=native={}", swift_lib.display());
    println!(
        "cargo:rustc-link-search=native={}",
        sdk.join("usr/lib/swift").display()
    );
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}
