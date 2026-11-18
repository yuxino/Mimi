//! Names for explicitly selected, development-only private evidence catalogs.
//! This module never reads the environment or touches the filesystem.

pub const EXTRA_WORKSPACE_LIMIT: usize = 8;
pub const WORKSPACE_NAME_LIMIT: usize = 48;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum EvidenceWorkspace {
    #[default]
    Default,
    Named(String),
}

impl EvidenceWorkspace {
    pub fn parse(value: Option<&str>) -> Result<Self, &'static str> {
        match value {
            None | Some("" | "default") => Ok(Self::Default),
            Some(name) if valid_workspace_name(name) => Ok(Self::Named(name.to_owned())),
            Some(_) => Err("development_evidence_workspace_invalid"),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Default => "default",
            Self::Named(name) => name,
        }
    }
}

pub fn valid_workspace_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= WORKSPACE_NAME_LIMIT
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_explicit_default_preserve_the_legacy_catalog() {
        for value in [None, Some(""), Some("default")] {
            let workspace = EvidenceWorkspace::parse(value).unwrap();
            assert_eq!(workspace, EvidenceWorkspace::Default);
            assert_eq!(workspace.label(), "default");
        }
    }

    #[test]
    fn named_catalogs_are_bounded_ascii_identifiers() {
        for value in ["batch-2", "JA_KO_03", "0", &"a".repeat(48)] {
            assert_eq!(
                EvidenceWorkspace::parse(Some(value)).unwrap().label(),
                value
            );
        }
        for value in [
            "-batch",
            "_batch",
            ".",
            "..",
            "../case",
            "case/nested",
            "case\\nested",
            "/absolute",
            "白",
            " space",
            "tab\t",
            "newline\n",
            "key=value",
            "c:drive",
            "default ",
            &"a".repeat(49),
        ] {
            assert_eq!(
                EvidenceWorkspace::parse(Some(value)),
                Err("development_evidence_workspace_invalid")
            );
        }
        assert_eq!(EXTRA_WORKSPACE_LIMIT + 1, 9);
    }
}
