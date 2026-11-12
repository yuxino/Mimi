//! Local transport exceptions for native platform adapters.

#[derive(Clone, Copy)]
pub enum HttpEndpointPolicy {
    /// The provider runtime's existing native-platform transport policy.
    Runtime,
    /// Android settings require a separate, explicit cleartext opt-in. This is
    /// an adapter policy, including when that adapter is tested through host JNI.
    Android { allow_local_http: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpEndpointError {
    Invalid,
    HttpsRequired,
}

/// One URL parser and API-path completion rule for both text protocols and
/// native configuration validation. Platform policy never changes the parser.
pub fn normalize_http_endpoint(
    value: &str,
    suffix: &str,
    policy: HttpEndpointPolicy,
) -> Result<url::Url, HttpEndpointError> {
    use HttpEndpointError::{HttpsRequired, Invalid};
    if value.len() > 2048 || value.chars().any(char::is_control) {
        return Err(Invalid);
    }
    let input = value.trim();
    let android = matches!(policy, HttpEndpointPolicy::Android { .. });
    // Preserve the Android editor's explicit URL shape. URL parsers otherwise
    // repair missing slashes, backslashes and an empty user-info component.
    if android
        && (!(input.starts_with("https://") || input.starts_with("http://"))
            || input.contains('\\')
            || input
                .split_once("://")
                .is_some_and(|(_, rest)| rest.split('/').next().unwrap_or("").contains('@')))
    {
        return Err(Invalid);
    }
    let mut url = url::Url::parse(input).map_err(|_| Invalid)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || (android && url.port() == Some(0))
    {
        return Err(Invalid);
    }
    let local = match policy {
        HttpEndpointPolicy::Runtime => endpoint_host_is_local(url.host_str()),
        HttpEndpointPolicy::Android { allow_local_http } => {
            let host = android_input_host(input)?;
            allow_local_http
                && matches!(
                    host.as_str(),
                    "localhost" | "127.0.0.1" | "[::1]" | "10.0.2.2"
                )
        }
    };
    if url.scheme() == "http" && !local {
        return Err(HttpsRequired);
    }
    let path = url.path().trim_end_matches('/');
    let completed = if path.ends_with(suffix) {
        path.to_owned()
    } else {
        format!("{path}{suffix}")
    };
    url.set_path(&completed);
    Ok(url)
}

/// Compare the host *before* WHATWG numeric-IPv4 repair. Android historically
/// allowed literal loopback hosts, not decimal, octal or shortened IP aliases.
/// IDNA and percent decoding still accept the same Unicode/encoded spellings.
fn android_input_host(input: &str) -> Result<String, HttpEndpointError> {
    use HttpEndpointError::Invalid;
    let authority = input
        .split_once("://")
        .ok_or(Invalid)?
        .1
        // Both URL parsers accept extra authority slashes. Skip only those;
        // numeric hosts must still be checked before WHATWG IPv4 repair.
        .trim_start_matches('/')
        .split(['/', '?', '#'])
        .next()
        .ok_or(Invalid)?;
    if let Some(rest) = authority.strip_prefix('[') {
        let host = rest.split_once(']').ok_or(Invalid)?.0;
        let address: std::net::Ipv6Addr = host.parse().map_err(|_| Invalid)?;
        return Ok(format!("[{address}]"));
    }
    let host = authority.split(':').next().ok_or(Invalid)?;
    let decoded = percent_encoding::percent_decode_str(host)
        .decode_utf8()
        .map_err(|_| Invalid)?;
    idna::domain_to_ascii(&decoded).map_err(|_| Invalid)
}

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
    fn android_endpoint_contracts_preserve_native_transport_and_input_rules() {
        let contract: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../translation-contracts.json"
        )))
        .unwrap();
        for case in contract["androidEndpoints"].as_array().unwrap() {
            let policy = HttpEndpointPolicy::Android {
                allow_local_http: case["allowLocalHttp"].as_bool().unwrap(),
            };
            let input = case["input"].as_str().unwrap();
            let result = if case["route"] == "deepLX" {
                crate::core::protocols::deeplx::endpoint_with_policy(input, policy)
            } else {
                crate::core::protocols::openai_compatible::endpoint_with_policy(input, policy)
            };
            if let Some(expected) = case["expected"].as_str() {
                assert_eq!(result.unwrap().as_str(), expected, "{}", case["id"]);
            } else {
                let expected = if case["error"] == "translation_https_required" {
                    HttpEndpointError::HttpsRequired
                } else {
                    HttpEndpointError::Invalid
                };
                assert_eq!(result, Err(expected), "{}", case["id"]);
            }
            if case["runtimeAligned"] == true {
                let runtime = if case["route"] == "deepLX" {
                    crate::core::protocols::deeplx::endpoint(input).ok()
                } else {
                    crate::core::protocols::openai_compatible::endpoint(input).ok()
                };
                assert_eq!(
                    runtime.as_ref().map(url::Url::as_str),
                    case["expected"].as_str(),
                    "runtime: {}",
                    case["id"]
                );
            }
        }
    }

    #[test]
    fn native_editor_policy_does_not_change_runtime_url_normalization() {
        for input in [
            "https:example.test/v1",
            "https://@example.test/v1",
            "https://example.test\\v1",
        ] {
            assert!(crate::core::protocols::openai_compatible::endpoint(input).is_ok());
            assert_eq!(
                normalize_http_endpoint(
                    input,
                    "/chat/completions",
                    HttpEndpointPolicy::Android {
                        allow_local_http: true
                    }
                ),
                Err(HttpEndpointError::Invalid)
            );
        }
        assert_eq!(
            crate::core::protocols::openai_compatible::endpoint("http://10.0.2.2/v1").is_ok(),
            cfg!(target_os = "android")
        );
    }

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
