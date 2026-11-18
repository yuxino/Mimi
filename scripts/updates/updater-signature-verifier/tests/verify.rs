use base64::Engine;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

// Public fixture signed by Tauri's CLI. The disposable private key was deleted.
const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDMxNTQ4QjQxMkJEM0U4QjcKUldTMzZOTXJRWXRVTWNPbEw5d0FRNG1QVU55Rnl0a2VZaSsvUktEZWJqTHVMNmNzZGxtL2p5VEIK";
const SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVTMzZOTXJRWXRVTVhLZ2YyTmIyRis5QS9GdFQwbE0vQ2ZmQThNMEJXQkg3d2dqN2U3Q1JPaXVGN1hLVG9QUlJuUW1sSjM5Zzk2WGREVHo5OFk3U0lNL3doRS9pZkhwUndBPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkxNDU1NTk4CWZpbGU6YXJ0aWZhY3QKaXc2UEw2VXI0a0RBSUFYSFl0OFZiL3puc3loWklrelJkMUpHR2ZpT0d2WkhHWUJ1bTJlN0ZrdXl3OWtMaTRxTjhNeTRxcnlqWnd6aWFybzFBblorQUE9PQo=";
static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mimi-signature-test-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("artifact"), vec![0x5a; 192 * 1024]).unwrap();
        std::fs::write(path.join("artifact.sig"), SIGNATURE).unwrap();
        Self(path)
    }

    fn verify(&self, public_key: &str, extra: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mimi-updater-signature-verifier"))
            .arg(public_key)
            .arg(self.0.join("artifact.sig"))
            .arg(self.0.join("artifact"))
            .args(extra)
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn accepts_tauri_signature_across_multiple_stream_chunks() {
    assert!(Fixture::new().verify(PUBLIC_KEY, &[]).status.success());
}

#[test]
fn rejects_tampered_and_truncated_artifacts() {
    let fixture = Fixture::new();
    let mut bytes = vec![0x5a; 192 * 1024];
    bytes[100_000] ^= 1;
    std::fs::write(fixture.0.join("artifact"), &bytes).unwrap();
    assert!(!fixture.verify(PUBLIC_KEY, &[]).status.success());
    std::fs::write(fixture.0.join("artifact"), &bytes[..64 * 1024]).unwrap();
    assert!(!fixture.verify(PUBLIC_KEY, &[]).status.success());
}

#[test]
fn rejects_wrong_public_key_with_the_same_key_id() {
    let engine = base64::engine::general_purpose::STANDARD;
    let text = String::from_utf8(engine.decode(PUBLIC_KEY).unwrap()).unwrap();
    let mut lines = text.lines();
    let comment = lines.next().unwrap();
    let mut key = engine.decode(lines.next().unwrap()).unwrap();
    // Preserve the algorithm and key ID; only alter the actual public key.
    key[20] ^= 1;
    let wrong_key = engine.encode(format!("{comment}\n{}\n", engine.encode(key)));
    assert!(!Fixture::new().verify(&wrong_key, &[]).status.success());
}

#[test]
fn rejects_missing_files_bad_encoding_and_extra_arguments() {
    let fixture = Fixture::new();
    assert!(!fixture.verify("invalid base64!", &[]).status.success());
    assert!(!fixture.verify(PUBLIC_KEY, &["unexpected"]).status.success());
    std::fs::write(fixture.0.join("artifact.sig"), "invalid signature!").unwrap();
    assert!(!fixture.verify(PUBLIC_KEY, &[]).status.success());
    std::fs::remove_file(fixture.0.join("artifact.sig")).unwrap();
    assert!(!fixture.verify(PUBLIC_KEY, &[]).status.success());
    let output = Command::new(env!("CARGO_BIN_EXE_mimi-updater-signature-verifier"))
        .output()
        .unwrap();
    assert!(!output.status.success());
}
