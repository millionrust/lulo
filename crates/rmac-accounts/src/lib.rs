//! Pure account, provider, discovery, and sign-in domain logic.
//! Network access, GOA, credentials storage, and UI belong to later adapters.

pub mod autoconfig;
pub mod model;
pub mod provider;
pub mod sign_in;

/// A value which must never appear in diagnostic output.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted]")
    }
}

impl std::fmt::Display for Secret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[redacted]")
    }
}

#[cfg(test)]
mod tests {
    use super::Secret;

    #[test]
    fn secrets_are_redacted() {
        let secret = Secret::new("planted-password".into());
        assert_eq!(format!("{secret:?} {secret}"), "[redacted] [redacted]");
    }
}
