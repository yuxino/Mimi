//! Local transport exceptions for native platform adapters.

/// Android's emulator maps the host machine's loopback to 10.0.2.2. Actual
/// cleartext permission is still checked by the Android platform adapter.
pub fn endpoint_host_is_local(host: Option<&str>) -> bool {
    matches!(host, Some("localhost" | "127.0.0.1" | "[::1]"))
        || (cfg!(target_os = "android") && host == Some("10.0.2.2"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_exception_is_exact_and_the_emulator_exception_is_android_only() {
        for host in ["localhost", "127.0.0.1", "[::1]"] {
            assert!(endpoint_host_is_local(Some(host)));
        }
        assert_eq!(
            endpoint_host_is_local(Some("10.0.2.2")),
            cfg!(target_os = "android")
        );
        for host in [
            None,
            Some("localhost.example.com"),
            Some("127.0.0.2"),
            Some("192.168.1.1"),
            Some("example.com"),
        ] {
            assert!(!endpoint_host_is_local(host));
        }
    }
}
