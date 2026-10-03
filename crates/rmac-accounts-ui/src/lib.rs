//! Shared, toolkit-independent presentation state for the Internet Accounts
//! sheet. Mail and Calendar can open the same workflow in their own windows.

use rmac_accounts::{model::email_domain, provider::Provider};
use rmac_accounts_linux::GoaAccount;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    ICloud,
    Microsoft,
    Google,
    Yahoo,
    OtherMail,
    OtherCalendar,
}

impl Choice {
    pub const ALL: [Self; 6] = [
        Self::ICloud,
        Self::Microsoft,
        Self::Google,
        Self::Yahoo,
        Self::OtherMail,
        Self::OtherCalendar,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::ICloud => "iCloud",
            Self::Microsoft => "Microsoft",
            Self::Google => "Google",
            Self::Yahoo => "Yahoo",
            Self::OtherMail => "Other Mail Account…",
            Self::OtherCalendar => "Other Calendar Account…",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Self::ICloud | Self::Yahoo => "App-specific password",
            Self::Microsoft => "Outlook, Hotmail, Microsoft 365",
            Self::Google => "Gmail, Google Workspace",
            Self::OtherMail => "IMAP and SMTP",
            Self::OtherCalendar => "CalDAV",
        }
    }

    pub fn provider(self) -> Provider {
        match self {
            Self::ICloud => Provider::ICloud,
            Self::Microsoft => Provider::Microsoft,
            Self::Google => Provider::Google,
            Self::Yahoo => Provider::Yahoo,
            Self::OtherMail | Self::OtherCalendar => Provider::Other,
        }
    }

    pub fn from_address(address: &str) -> Option<Self> {
        let domain = email_domain(address)?;
        Some(match Provider::from_domain(domain) {
            Provider::ICloud => Self::ICloud,
            Provider::Microsoft => Self::Microsoft,
            Provider::Google => Self::Google,
            Provider::Yahoo => Self::Yahoo,
            Provider::Other => Self::OtherMail,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Choose,
    Credentials,
    Browser,
    Services,
}

#[derive(Clone)]
pub struct AddSheet {
    pub step: Step,
    pub choice: Option<Choice>,
    pub address: String,
    pub name: String,
    pub password: String,
    pub caldav_uri: String,
    pub manual_imap: String,
    pub manual_smtp: String,
    pub services: rmac_accounts::model::Services,
    pub error: Option<&'static str>,
}

impl std::fmt::Debug for AddSheet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AddSheet")
            .field("step", &self.step)
            .field("choice", &self.choice)
            .field("address", &"[redacted]")
            .field("name", &"[redacted]")
            .field("password", &"[redacted]")
            .finish()
    }
}

impl Default for AddSheet {
    fn default() -> Self {
        Self {
            step: Step::Choose,
            choice: None,
            address: String::new(),
            name: String::new(),
            password: String::new(),
            caldav_uri: String::new(),
            manual_imap: String::new(),
            manual_smtp: String::new(),
            services: rmac_accounts::model::Services::ALL,
            error: None,
        }
    }
}

impl AddSheet {
    pub fn continue_choice(&mut self) -> bool {
        let choice = self.choice.or_else(|| Choice::from_address(&self.address));
        let Some(choice) = choice else {
            self.error = Some("Enter a valid email address or choose a provider.");
            return false;
        };
        if !self.address.trim().is_empty() && email_domain(&self.address).is_none() {
            self.error = Some("Enter a valid email address.");
            return false;
        }
        self.choice = Some(choice);
        self.services = match choice {
            Choice::OtherMail => rmac_accounts::model::Services::MAIL_ONLY,
            Choice::OtherCalendar => rmac_accounts::model::Services {
                mail: false,
                calendar: true,
                contacts: false,
            },
            _ => rmac_accounts::model::Services::ALL,
        };
        self.step = if choice.provider().info().oauth.is_some() {
            Step::Browser
        } else {
            Step::Credentials
        };
        self.error = None;
        true
    }

    pub fn credentials_valid(&self) -> bool {
        email_domain(&self.address).is_some()
            && !self.name.trim().is_empty()
            && !self.password.is_empty()
            && (self.choice != Some(Choice::OtherCalendar)
                || self.caldav_uri.starts_with("https://"))
    }
}

pub fn provider_label(account: &GoaAccount) -> &'static str {
    match account.provider.as_str() {
        "google" => "Google",
        "ms_graph" => "Microsoft",
        "webdav" => "Calendar Account",
        "imap_smtp" => "Mail Account",
        _ => "Internet Account",
    }
}

pub fn service_summary(account: &GoaAccount) -> String {
    let mut names = Vec::new();
    if account.services.mail {
        names.push("Mail");
    }
    if account.services.calendar {
        names.push("Calendars");
    }
    if account.services.contacts {
        names.push("Contacts");
    }
    if names.is_empty() {
        "No services enabled".to_owned()
    } else {
        names.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooser_uses_address_or_explicit_provider() {
        let mut sheet = AddSheet {
            address: "person@gmail.com".into(),
            ..Default::default()
        };
        assert!(sheet.continue_choice());
        assert_eq!(sheet.choice, Some(Choice::Google));
        assert_eq!(sheet.step, Step::Browser);
        sheet = AddSheet {
            choice: Some(Choice::OtherCalendar),
            ..Default::default()
        };
        assert!(sheet.continue_choice());
        assert_eq!(sheet.step, Step::Credentials);
    }

    #[test]
    fn credentials_require_identity_and_password() {
        let mut sheet = AddSheet::default();
        assert!(!sheet.credentials_valid());
        sheet.address = "person@example.com".into();
        sheet.name = "Person".into();
        sheet.password = "fake".into();
        assert!(sheet.credentials_valid());
        sheet.choice = Some(Choice::OtherCalendar);
        assert!(!sheet.credentials_valid());
        sheet.caldav_uri = "https://calendar.example.com/".into();
        assert!(sheet.credentials_valid());
    }
}
