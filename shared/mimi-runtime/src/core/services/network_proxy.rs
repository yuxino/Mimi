//! Persisted proxy choices without credentials, network access or OS state.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProxyMode {
    Direct,
    #[default]
    System,
    Custom,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProxyConfig {
    pub mode: ProxyMode,
    pub url: Option<String>,
}

impl std::fmt::Debug for ProxyConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProxyConfig")
            .field("mode", &self.mode)
            .field("has_url", &self.url.is_some())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProxyConfigError {
    #[error("network_proxy_invalid_url")]
    InvalidUrl,
    #[error("network_proxy_unsupported_scheme")]
    UnsupportedScheme,
    #[error("network_proxy_authentication_unsupported")]
    AuthenticationUnsupported,
    #[error("network_proxy_builder_failed")]
    BuilderFailed,
    #[error("provider_trust_roots_invalid")]
    InvalidTrustRoots,
}

impl ProxyConfig {
    /// Normalize only the selected route. Unused custom addresses are removed
    /// so snapshots/preferences never retain a proxy secret or stale address.
    pub fn validate(&self) -> Result<Self, ProxyConfigError> {
        if self.mode != ProxyMode::Custom {
            return Ok(Self {
                mode: self.mode,
                url: None,
            });
        }
        let endpoint =
            validated_proxy_url(self.url.as_deref().ok_or(ProxyConfigError::InvalidUrl)?)?;
        Ok(Self {
            mode: self.mode,
            url: Some(endpoint.into()),
        })
    }
}

pub fn validated_proxy_url(value: &str) -> Result<url::Url, ProxyConfigError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 2_048 || value.chars().any(char::is_control) {
        return Err(ProxyConfigError::InvalidUrl);
    }
    let mut endpoint = url::Url::parse(value).map_err(|_| ProxyConfigError::InvalidUrl)?;
    if !endpoint.username().is_empty() || endpoint.password().is_some() {
        return Err(ProxyConfigError::AuthenticationUnsupported);
    }
    if !matches!(endpoint.scheme(), "http" | "socks5" | "socks5h") {
        return Err(ProxyConfigError::UnsupportedScheme);
    }
    if endpoint.host().is_none()
        || !matches!(endpoint.path(), "" | "/")
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || endpoint.port() == Some(0)
    {
        return Err(ProxyConfigError::InvalidUrl);
    }
    if endpoint.port().is_none() && matches!(endpoint.scheme(), "socks5" | "socks5h") {
        endpoint
            .set_port(Some(1080))
            .map_err(|_| ProxyConfigError::InvalidUrl)?;
    }
    Ok(endpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_defaults_and_wire_choices_are_stable() {
        assert_eq!(
            serde_json::from_str::<ProxyConfig>("{}").unwrap(),
            ProxyConfig::default()
        );
        for mode in [ProxyMode::Direct, ProxyMode::System, ProxyMode::Custom] {
            let config = ProxyConfig {
                mode,
                url: Some("http://127.0.0.1:7890".into()),
            };
            let value = serde_json::to_value(&config).unwrap();
            assert_eq!(
                serde_json::from_value::<ProxyConfig>(value).unwrap(),
                config
            );
        }
    }

    #[test]
    fn validates_only_credential_free_proxy_endpoints() {
        for endpoint in [
            "http://127.0.0.1:7890",
            "socks5://localhost",
            "socks5h://[::1]:1080",
        ] {
            assert!(validated_proxy_url(endpoint).is_ok());
        }
        assert_eq!(
            validated_proxy_url("socks5://localhost").unwrap().port(),
            Some(1080)
        );
        for endpoint in [
            "http://proxy:0",
            "http://proxy/translate",
            "http://proxy/?key=private",
            "http://proxy/#private",
            "http://proxy\nbad",
            "not a url",
        ] {
            assert_eq!(
                validated_proxy_url(endpoint),
                Err(ProxyConfigError::InvalidUrl)
            );
        }
        assert_eq!(
            validated_proxy_url("http://user:private@proxy:7890"),
            Err(ProxyConfigError::AuthenticationUnsupported)
        );
        assert_eq!(
            validated_proxy_url("https://proxy:7890"),
            Err(ProxyConfigError::UnsupportedScheme)
        );
        assert_eq!(
            validated_proxy_url("socks4://proxy:1080"),
            Err(ProxyConfigError::UnsupportedScheme)
        );
    }

    #[test]
    fn inactive_addresses_are_not_retained_and_debug_does_not_disclose_them() {
        let config = ProxyConfig {
            mode: ProxyMode::Direct,
            url: Some("http://private-host:7890".into()),
        };
        assert!(config.validate().unwrap().url.is_none());
        let debug = format!("{config:?}");
        assert!(!debug.contains("private-host"));
        assert!(!ProxyConfigError::InvalidUrl.to_string().contains("private"));
    }
}
