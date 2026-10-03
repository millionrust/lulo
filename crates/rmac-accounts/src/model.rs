use crate::provider::Provider;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Service {
    Mail,
    Calendar,
    Contacts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Services {
    pub mail: bool,
    pub calendar: bool,
    pub contacts: bool,
}

impl Services {
    pub const ALL: Self = Self {
        mail: true,
        calendar: true,
        contacts: true,
    };

    pub const MAIL_ONLY: Self = Self {
        mail: true,
        calendar: false,
        contacts: false,
    };

    pub fn enabled(self, service: Service) -> bool {
        match service {
            Service::Mail => self.mail,
            Service::Calendar => self.calendar,
            Service::Contacts => self.contacts,
        }
    }

    pub fn set(&mut self, service: Service, enabled: bool) {
        match service {
            Service::Mail => self.mail = enabled,
            Service::Calendar => self.calendar = enabled,
            Service::Contacts => self.contacts = enabled,
        }
    }

    pub fn any(self) -> bool {
        self.mail || self.calendar || self.contacts
    }

    pub fn for_provider(provider: Provider) -> Self {
        match provider {
            Provider::Other => Self::MAIL_ONLY,
            _ => Self::ALL,
        }
    }
}

/// GOA object IDs are opaque. A Yahoo or iCloud Lulo account can own two IDs.
#[derive(Clone, PartialEq, Eq)]
pub struct Account {
    pub provider: Provider,
    pub display_name: String,
    pub address: String,
    pub goa_ids: Vec<String>,
    pub services: Services,
}

impl std::fmt::Debug for Account {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Account")
            .field("provider", &self.provider)
            .field("display_name", &"[redacted]")
            .field("address", &"[redacted]")
            .field("goa_ids", &"[redacted]")
            .field("services", &self.services)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountError {
    EmptyName,
    InvalidAddress,
    MissingGoaId,
    DuplicateGoaId,
    UnsupportedService,
}

impl Account {
    pub fn new(
        provider: Provider,
        display_name: String,
        address: String,
        goa_ids: Vec<String>,
        services: Services,
    ) -> Result<Self, AccountError> {
        if display_name.trim().is_empty() {
            return Err(AccountError::EmptyName);
        }
        if email_domain(&address).is_none() {
            return Err(AccountError::InvalidAddress);
        }
        if goa_ids.is_empty() || goa_ids.iter().any(|id| id.trim().is_empty()) {
            return Err(AccountError::MissingGoaId);
        }
        for (index, id) in goa_ids.iter().enumerate() {
            if goa_ids[..index].contains(id) {
                return Err(AccountError::DuplicateGoaId);
            }
        }
        if provider == Provider::Other && services.contacts {
            return Err(AccountError::UnsupportedService);
        }
        Ok(Self {
            provider,
            display_name,
            address,
            goa_ids,
            services,
        })
    }
}

/// A deliberately conservative address check for the chooser, not an RFC 5322 parser.
pub fn email_domain(address: &str) -> Option<&str> {
    let (local, domain) = address.trim().rsplit_once('@')?;
    if local.is_empty()
        || local.contains('@')
        || local.chars().any(char::is_whitespace)
        || domain.is_empty()
        || !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
        || domain.split('.').any(|label| {
            label.is_empty()
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        None
    } else {
        Some(domain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_validation_rejects_invalid_external_data() {
        for address in [
            "",
            "a",
            "a@@example.com",
            "a@.com",
            "a@example..com",
            "a@-example.com",
            "a@localhost",
            "a b@example.com",
        ] {
            assert_eq!(email_domain(address), None, "{address}");
        }
        assert_eq!(email_domain("me@Example.COM"), Some("Example.COM"));
    }

    #[test]
    fn service_switches_are_independent() {
        let mut services = Services::ALL;
        services.set(Service::Calendar, false);
        assert!(services.enabled(Service::Mail));
        assert!(!services.enabled(Service::Calendar));
        assert!(services.enabled(Service::Contacts));
    }

    #[test]
    fn grouped_account_needs_distinct_goa_ids() {
        let create = |ids| {
            Account::new(
                Provider::ICloud,
                "Personal".into(),
                "me@icloud.com".into(),
                ids,
                Services::ALL,
            )
        };
        assert!(create(vec!["1".into(), "2".into()]).is_ok());
        assert_eq!(
            create(vec!["1".into(), "1".into()]),
            Err(AccountError::DuplicateGoaId)
        );
        assert_eq!(create(vec![]), Err(AccountError::MissingGoaId));
    }

    #[test]
    fn account_debug_omits_personal_data() {
        let account = Account::new(
            Provider::Google,
            "Planted Name".into(),
            "planted@example.com".into(),
            vec!["planted-goa-id".into()],
            Services::ALL,
        )
        .expect("valid account");
        let debug = format!("{account:?}");
        assert!(!debug.contains("Planted Name"));
        assert!(!debug.contains("planted@example.com"));
        assert!(!debug.contains("planted-goa-id"));
    }
}
