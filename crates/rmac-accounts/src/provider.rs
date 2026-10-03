//! Provider data is pinned to GOA 3.58.0. ACC-2 must compare OAuth fields with
//! the installed libgoa-backend before using them to start a sign-in.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    Google,
    Microsoft,
    Yahoo,
    ICloud,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OAuthConfig {
    pub client_id: &'static str,
    /// GOA's embedded desktop-client value; not a user credential.
    pub client_secret: &'static str,
    pub authorization_uri: &'static str,
    pub token_uri: &'static str,
    pub redirect_uri: &'static str,
    pub scopes: &'static str,
    pub identity_uri: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailTransport {
    ImapSmtp,
    Graph,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketSecurity {
    Tls,
    StartTls,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerPreset {
    pub imap_host: &'static str,
    pub imap_port: u16,
    pub imap_security: SocketSecurity,
    pub smtp_host: &'static str,
    pub smtp_port: u16,
    pub smtp_security: SocketSecurity,
    pub caldav_uri: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderInfo {
    pub provider: Provider,
    pub goa_ids: &'static [&'static str],
    pub oauth: Option<OAuthConfig>,
    pub mail_transport: MailTransport,
    pub servers: Option<ServerPreset>,
    pub needs_app_password: bool,
}

const GOOGLE_OAUTH: OAuthConfig = OAuthConfig {
    client_id: "44438659992-7kgjeitenc16ssihbtdjbgguch7ju55s.apps.googleusercontent.com",
    client_secret: "-gMLuQyDiI0XrQS_vx_mhuYF",
    authorization_uri: "https://accounts.google.com/o/oauth2/v2/auth",
    token_uri: "https://oauth2.googleapis.com/token",
    redirect_uri: "com.googleusercontent.apps.44438659992-7kgjeitenc16ssihbtdjbgguch7ju55s:/oauth2redirect",
    scopes: "https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/calendar https://www.google.com/m8/feeds/ https://www.googleapis.com/auth/carddav https://mail.google.com/ https://www.googleapis.com/auth/tasks",
    identity_uri: "https://www.googleapis.com/oauth2/v2/userinfo",
};

const MICROSOFT_OAUTH: OAuthConfig = OAuthConfig {
    client_id: "b155a604-3c31-4079-93b0-6bb6aa9d5464",
    client_secret: "",
    authorization_uri: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
    token_uri: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
    redirect_uri: "goa-oauth2://localhost/b155a604-3c31-4079-93b0-6bb6aa9d5464",
    scopes: "offline_access calendars.readwrite calendars.readwrite.shared contacts.readwrite contacts.readwrite.shared files.readwrite files.readwrite.all mail.readwrite mail.readwrite.shared mail.send mail.send.shared mailboxsettings.read people.read sites.read.all sites.readwrite.all tasks.readwrite tasks.readwrite.shared user.read user.readbasic.all",
    identity_uri: "https://graph.microsoft.com/v1.0/me",
};

pub const PROVIDERS: [ProviderInfo; 5] = [
    ProviderInfo {
        provider: Provider::Google,
        goa_ids: &["google"],
        oauth: Some(GOOGLE_OAUTH),
        mail_transport: MailTransport::ImapSmtp,
        servers: Some(ServerPreset {
            imap_host: "imap.gmail.com",
            imap_port: 993,
            imap_security: SocketSecurity::Tls,
            smtp_host: "smtp.gmail.com",
            smtp_port: 587,
            smtp_security: SocketSecurity::StartTls,
            caldav_uri: None, // GOA builds a per-account CalDAV URI.
        }),
        needs_app_password: false,
    },
    ProviderInfo {
        provider: Provider::Microsoft,
        goa_ids: &["ms_graph"],
        oauth: Some(MICROSOFT_OAUTH),
        mail_transport: MailTransport::Graph,
        servers: None,
        needs_app_password: false,
    },
    ProviderInfo {
        provider: Provider::Yahoo,
        goa_ids: &["imap_smtp", "webdav"],
        oauth: None,
        mail_transport: MailTransport::ImapSmtp,
        servers: Some(ServerPreset {
            imap_host: "imap.mail.yahoo.com",
            imap_port: 993,
            imap_security: SocketSecurity::Tls,
            smtp_host: "smtp.mail.yahoo.com",
            smtp_port: 465,
            smtp_security: SocketSecurity::Tls,
            caldav_uri: Some("https://caldav.calendar.yahoo.com"),
        }),
        needs_app_password: true,
    },
    ProviderInfo {
        provider: Provider::ICloud,
        goa_ids: &["imap_smtp", "webdav"],
        oauth: None,
        mail_transport: MailTransport::ImapSmtp,
        servers: Some(ServerPreset {
            imap_host: "imap.mail.me.com",
            imap_port: 993,
            imap_security: SocketSecurity::Tls,
            smtp_host: "smtp.mail.me.com",
            smtp_port: 587,
            smtp_security: SocketSecurity::StartTls,
            caldav_uri: Some("https://caldav.icloud.com"),
        }),
        needs_app_password: true,
    },
    ProviderInfo {
        provider: Provider::Other,
        goa_ids: &["imap_smtp", "webdav"],
        oauth: None,
        mail_transport: MailTransport::ImapSmtp,
        servers: None,
        needs_app_password: false,
    },
];

impl Provider {
    pub fn info(self) -> &'static ProviderInfo {
        &PROVIDERS[self as usize]
    }

    pub fn from_domain(domain: &str) -> Self {
        match domain
            .trim()
            .trim_end_matches('.')
            .to_ascii_lowercase()
            .as_str()
        {
            "gmail.com" | "googlemail.com" => Self::Google,
            "outlook.com" | "hotmail.com" | "hotmail.co.uk" | "live.com" | "msn.com" => {
                Self::Microsoft
            }
            "yahoo.com" | "yahoo.co.uk" | "ymail.com" | "rocketmail.com" => Self::Yahoo,
            "icloud.com" | "me.com" | "mac.com" => Self::ICloud,
            _ => Self::Other,
        }
    }

    /// Only a verified provider-owned MX host should trigger an OAuth switch.
    pub fn from_mx_host(host: &str) -> Option<Self> {
        let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
        if host == "aspmx.l.google.com" || host.ends_with(".aspmx.l.google.com") {
            Some(Self::Google)
        } else if host.ends_with(".mail.protection.outlook.com") {
            Some(Self::Microsoft)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_mapping_is_case_insensitive_and_exact() {
        assert_eq!(Provider::from_domain(" GMAIL.COM. "), Provider::Google);
        assert_eq!(Provider::from_domain("hotmail.com"), Provider::Microsoft);
        assert_eq!(Provider::from_domain("ymail.com"), Provider::Yahoo);
        assert_eq!(Provider::from_domain("me.com"), Provider::ICloud);
        assert_eq!(Provider::from_domain("gmail.com.evil"), Provider::Other);
    }

    #[test]
    fn mx_mapping_does_not_trust_suffix_lookalikes() {
        assert_eq!(
            Provider::from_mx_host("aspmx.l.google.com."),
            Some(Provider::Google)
        );
        assert_eq!(
            Provider::from_mx_host("foo.mail.protection.outlook.com"),
            Some(Provider::Microsoft)
        );
        assert_eq!(
            Provider::from_mx_host("mail.protection.outlook.com.evil"),
            None
        );
    }

    #[test]
    fn goa_provider_ids_and_oauth_scopes_are_pinned() {
        assert_eq!(Provider::Google.info().goa_ids, &["google"]);
        assert_eq!(Provider::Microsoft.info().goa_ids, &["ms_graph"]);
        assert_eq!(Provider::Yahoo.info().goa_ids, &["imap_smtp", "webdav"]);
        assert!(GOOGLE_OAUTH.scopes.contains("https://mail.google.com/"));
        assert_eq!(GOOGLE_OAUTH.client_secret, "-gMLuQyDiI0XrQS_vx_mhuYF");
        assert!(MICROSOFT_OAUTH.scopes.contains("mail.readwrite"));
        assert!(!MICROSOFT_OAUTH.scopes.contains("IMAP"));
        assert_eq!(
            Provider::Microsoft.info().mail_transport,
            MailTransport::Graph
        );
        assert_eq!(
            Provider::Google
                .info()
                .servers
                .map(|servers| servers.smtp_security),
            Some(SocketSecurity::StartTls)
        );
    }
}
