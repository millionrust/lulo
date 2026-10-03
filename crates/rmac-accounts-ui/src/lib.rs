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

    pub fn password_help_url(self) -> Option<&'static str> {
        match self {
            Self::ICloud => Some("https://account.apple.com/"),
            Self::Yahoo => Some("https://login.yahoo.com/account/security"),
            _ => None,
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
    Discovering,
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
            Choice::ICloud | Choice::Yahoo => rmac_accounts::model::Services {
                mail: true,
                calendar: true,
                contacts: false,
            },
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

/// One visible account. Password providers use two GOA objects, one for mail
/// and one for CalDAV, but have one row and one delete action in Settings.
#[derive(Clone)]
pub struct AccountRow {
    pub identity: String,
    pub label: &'static str,
    pub paths: Vec<String>,
    pub services: rmac_accounts::model::Services,
    pub mail_path: Option<String>,
    pub calendar_path: Option<String>,
    pub contacts_path: Option<String>,
}

impl AccountRow {
    pub fn service_path(&self, service: rmac_accounts::model::Service) -> Option<&str> {
        use rmac_accounts::model::Service;
        match service {
            Service::Mail => self.mail_path.as_deref(),
            Service::Calendar => self.calendar_path.as_deref(),
            Service::Contacts => self.contacts_path.as_deref(),
        }
    }

    pub fn summary(&self) -> String {
        let account = GoaAccount {
            path: String::new(),
            id: String::new(),
            provider: String::new(),
            identity: String::new(),
            services: self.services,
        };
        service_summary(&account)
    }
}

pub fn account_rows(accounts: &[GoaAccount]) -> Vec<AccountRow> {
    let mut rows = Vec::new();
    let mut used = vec![false; accounts.len()];
    for (index, account) in accounts.iter().enumerate() {
        if used[index] {
            continue;
        }
        used[index] = true;
        let choice = Choice::from_address(&account.identity);
        let pair = if matches!(choice, Some(Choice::ICloud | Choice::Yahoo))
            && matches!(account.provider.as_str(), "imap_smtp" | "webdav")
        {
            let candidates: Vec<_> = accounts
                .iter()
                .enumerate()
                .filter(|(other_index, other)| {
                    !used[*other_index]
                        && other.identity == account.identity
                        && matches!(
                            (account.provider.as_str(), other.provider.as_str()),
                            ("imap_smtp", "webdav") | ("webdav", "imap_smtp")
                        )
                })
                .collect();
            if candidates.len() == 1 {
                Some(candidates[0])
            } else {
                None
            }
        } else {
            None
        };
        let mut members = vec![account];
        if let Some((other_index, other)) = pair {
            used[other_index] = true;
            members.push(other);
        }
        let services = rmac_accounts::model::Services {
            mail: members.iter().any(|item| item.services.mail),
            calendar: members.iter().any(|item| item.services.calendar),
            contacts: members.iter().any(|item| item.services.contacts),
        };
        let find_path = |provider: &str| {
            members
                .iter()
                .find(|item| item.provider == provider)
                .map(|item| item.path.clone())
        };
        let oauth = matches!(account.provider.as_str(), "google" | "ms_graph");
        let oauth_path = oauth.then(|| account.path.clone());
        rows.push(AccountRow {
            identity: account.identity.clone(),
            label: match choice {
                Some(Choice::ICloud)
                    if matches!(account.provider.as_str(), "imap_smtp" | "webdav") =>
                {
                    "iCloud"
                }
                Some(Choice::Yahoo)
                    if matches!(account.provider.as_str(), "imap_smtp" | "webdav") =>
                {
                    "Yahoo"
                }
                _ => provider_label(account),
            },
            paths: members.iter().map(|item| item.path.clone()).collect(),
            services,
            mail_path: find_path("imap_smtp").or_else(|| oauth_path.clone()),
            calendar_path: find_path("webdav").or_else(|| oauth_path.clone()),
            contacts_path: find_path("webdav")
                .filter(|_| {
                    members
                        .iter()
                        .any(|item| item.provider == "webdav" && item.services.contacts)
                })
                .or(oauth_path),
        });
    }
    rows
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

    #[test]
    fn password_provider_goa_objects_share_one_row() {
        use rmac_accounts::model::Services;
        let account = |path: &str, provider: &str, services| GoaAccount {
            path: path.into(),
            id: path.into(),
            provider: provider.into(),
            identity: "person@icloud.com".into(),
            services,
        };
        let rows = account_rows(&[
            account("/mail", "imap_smtp", Services::MAIL_ONLY),
            account(
                "/calendar",
                "webdav",
                Services {
                    mail: false,
                    calendar: true,
                    contacts: false,
                },
            ),
        ]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "iCloud");
        assert_eq!(rows[0].summary(), "Mail, Calendars");
        assert_eq!(
            rows[0].service_path(rmac_accounts::model::Service::Mail),
            Some("/mail")
        );
        assert_eq!(
            rows[0].service_path(rmac_accounts::model::Service::Calendar),
            Some("/calendar")
        );
        assert_eq!(rows[0].paths.len(), 2);
    }

    #[test]
    fn ambiguous_password_accounts_stay_separate() {
        use rmac_accounts::model::Services;
        let account = |path: &str, provider: &str| GoaAccount {
            path: path.into(),
            id: path.into(),
            provider: provider.into(),
            identity: "person@yahoo.com".into(),
            services: Services::MAIL_ONLY,
        };
        let rows = account_rows(&[
            account("/mail", "imap_smtp"),
            account("/calendar-a", "webdav"),
            account("/calendar-b", "webdav"),
        ]);
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|row| row.paths.len() == 1));
    }
}
