use quick_xml::{events::Event, Reader};

pub use crate::provider::SocketSecurity;
use crate::{model::email_domain, provider::Provider};

const MAX_XML_BYTES: usize = 256 * 1024;
const MAX_XML_EVENTS: usize = 4096;
const MAX_XML_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailServer {
    pub host: String,
    pub port: u16,
    pub security: SocketSecurity,
    pub username: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailConfig {
    pub imap: MailServer,
    pub smtp: MailServer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoconfigError {
    InvalidAddress,
    TooLarge,
    MalformedXml,
    DomainMismatch,
    NoSecureServers,
}

#[derive(Debug, Default)]
struct Candidate {
    kind: String,
    host: String,
    port: String,
    socket: String,
    username: String,
    authentication: String,
}

impl Candidate {
    fn server(self, address: &str) -> Option<MailServer> {
        let security = match self.socket.to_ascii_uppercase().as_str() {
            "SSL" | "TLS" => SocketSecurity::Tls,
            "STARTTLS" => SocketSecurity::StartTls,
            _ => return None,
        };
        let port = self.port.parse::<u16>().ok().filter(|port| *port != 0)?;
        if self.authentication.to_ascii_lowercase() != "password-cleartext" {
            return None;
        }
        let host = self.host.trim().to_ascii_lowercase();
        if !valid_hostname(&host) {
            return None;
        }
        let username = expand_username(&self.username, address)?;
        Some(MailServer {
            host,
            port,
            security,
            username,
        })
    }
}

fn valid_hostname(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.contains('.')
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn expand_username(template: &str, address: &str) -> Option<String> {
    let (local, domain) = address.rsplit_once('@')?;
    let value = template
        .replace("%EMAILADDRESS%", address)
        .replace("%EMAILLOCALPART%", local)
        .replace("%EMAILDOMAIN%", domain);
    if value.is_empty() || value.contains('%') || value.chars().any(char::is_control) {
        None
    } else {
        Some(value)
    }
}

/// Parse Thunderbird/ISPDB v1.1 mail settings. Only secure IMAP and SMTP pairs
/// are returned. Callers must fetch XML over verified HTTPS with a size limit.
pub fn parse_ispdb(xml: &str, address: &str) -> Result<MailConfig, AutoconfigError> {
    let domain = email_domain(address).ok_or(AutoconfigError::InvalidAddress)?;
    if xml.len() > MAX_XML_BYTES {
        return Err(AutoconfigError::TooLarge);
    }
    let mut reader = Reader::from_str(xml);
    reader.config_mut().check_end_names = true;
    let mut depth = 0_usize;
    let mut events = 0_usize;
    let mut root = false;
    let mut provider = false;
    let mut matched_domain = false;
    let mut active: Option<(usize, Candidate)> = None;
    let mut field: Option<Vec<u8>> = None;
    let mut incoming = Vec::new();
    let mut outgoing = Vec::new();

    loop {
        events += 1;
        if events > MAX_XML_EVENTS {
            return Err(AutoconfigError::TooLarge);
        }
        match reader
            .read_event()
            .map_err(|_| AutoconfigError::MalformedXml)?
        {
            Event::Start(start) => {
                depth += 1;
                if depth > MAX_XML_DEPTH {
                    return Err(AutoconfigError::TooLarge);
                }
                let name = start.local_name();
                let name = name.as_ref();
                if depth == 1 {
                    root = name == b"clientConfig";
                    if !root {
                        return Err(AutoconfigError::MalformedXml);
                    }
                } else if depth == 2 && name == b"emailProvider" {
                    provider = true;
                } else if provider
                    && depth == 3
                    && matches!(name, b"incomingServer" | b"outgoingServer")
                {
                    let mut candidate = Candidate::default();
                    for attribute in start.attributes().with_checks(true) {
                        let attribute = attribute.map_err(|_| AutoconfigError::MalformedXml)?;
                        if attribute.key.as_ref() == b"type" {
                            candidate.kind = attribute
                                .decode_and_unescape_value(reader.decoder())
                                .map_err(|_| AutoconfigError::MalformedXml)?
                                .into_owned();
                        }
                    }
                    active = Some((depth, candidate));
                } else if (active.is_some() && depth == 4)
                    || (provider && depth == 3 && name == b"domain")
                {
                    field = Some(name.to_vec());
                }
            }
            Event::Text(value) => {
                if let Some(field_name) = field.as_deref() {
                    let text = value.decode().map_err(|_| AutoconfigError::MalformedXml)?;
                    let text = text.trim();
                    if field_name == b"domain" {
                        matched_domain |= text.eq_ignore_ascii_case(domain);
                    } else if let Some((_, candidate)) = active.as_mut() {
                        match field_name {
                            b"hostname" => candidate.host.push_str(text),
                            b"port" => candidate.port.push_str(text),
                            b"socketType" => candidate.socket.push_str(text),
                            b"username" => candidate.username.push_str(text),
                            b"authentication" => candidate.authentication.push_str(text),
                            _ => {}
                        }
                    }
                }
            }
            Event::End(end) => {
                let name = end.local_name();
                let name = name.as_ref();
                if field.as_deref() == Some(name) {
                    field = None;
                }
                if let Some((server_depth, _)) = &active {
                    if depth == *server_depth {
                        let (_, candidate) = active.take().ok_or(AutoconfigError::MalformedXml)?;
                        let kind = candidate.kind.clone();
                        if let Some(server) = candidate.server(address) {
                            match (name, kind.as_str()) {
                                (b"incomingServer", "imap") => incoming.push(server),
                                (b"outgoingServer", "smtp") => outgoing.push(server),
                                _ => {}
                            }
                        }
                    }
                }
                depth = depth.checked_sub(1).ok_or(AutoconfigError::MalformedXml)?;
            }
            Event::DocType(_) | Event::PI(_) => return Err(AutoconfigError::MalformedXml),
            Event::Empty(_) => {}
            Event::Eof => break,
            _ => {}
        }
    }
    if !root || !provider || depth != 0 {
        return Err(AutoconfigError::MalformedXml);
    }
    if !matched_domain {
        return Err(AutoconfigError::DomainMismatch);
    }
    let choose = |servers: Vec<MailServer>| {
        servers
            .into_iter()
            .min_by_key(|server| match server.security {
                SocketSecurity::Tls => 0,
                SocketSecurity::StartTls => 1,
            })
    };
    match (choose(incoming), choose(outgoing)) {
        (Some(imap), Some(smtp)) => Ok(MailConfig { imap, smtp }),
        _ => Err(AutoconfigError::NoSecureServers),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryStep {
    BuiltIn(Provider),
    AutoconfigHost,
    Ispdb,
    Mx,
    Manual,
    Configured,
    OAuth(Provider),
}

/// A nonblocking decision sequence. The Linux adapter performs each requested
/// network lookup and calls the corresponding method once it finishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovery {
    pub address: String,
    pub step: DiscoveryStep,
    pub config: Option<MailConfig>,
}

impl Discovery {
    pub fn new(address: String) -> Result<Self, AutoconfigError> {
        let domain = email_domain(&address).ok_or(AutoconfigError::InvalidAddress)?;
        let provider = Provider::from_domain(domain);
        let step = if provider == Provider::Other {
            DiscoveryStep::AutoconfigHost
        } else {
            DiscoveryStep::BuiltIn(provider)
        };
        Ok(Self {
            address,
            step,
            config: None,
        })
    }

    pub fn accept_builtin(&mut self) {
        if let DiscoveryStep::BuiltIn(provider) = self.step {
            self.step = if provider.info().oauth.is_some() {
                DiscoveryStep::OAuth(provider)
            } else {
                DiscoveryStep::Configured
            };
        }
    }

    pub fn autoconfig_result(&mut self, config: Option<MailConfig>) {
        if self.step == DiscoveryStep::AutoconfigHost {
            self.advance_config(config, DiscoveryStep::Ispdb);
        }
    }

    pub fn ispdb_result(&mut self, config: Option<MailConfig>) {
        if self.step == DiscoveryStep::Ispdb {
            self.advance_config(config, DiscoveryStep::Mx);
        }
    }

    fn advance_config(&mut self, config: Option<MailConfig>, otherwise: DiscoveryStep) {
        if let Some(config) = config {
            self.config = Some(config);
            self.step = DiscoveryStep::Configured;
        } else {
            self.step = otherwise;
        }
    }

    pub fn mx_result(&mut self, hosts: &[String]) {
        if self.step != DiscoveryStep::Mx {
            return;
        }
        self.step = hosts
            .iter()
            .find_map(|host| Provider::from_mx_host(host))
            .map_or(DiscoveryStep::Manual, DiscoveryStep::OAuth);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<clientConfig version="1.1"><emailProvider id="example.com"><domain>example.com</domain><incomingServer type="pop3"><hostname>pop.example.com</hostname><port>995</port><socketType>SSL</socketType><username>%EMAILADDRESS%</username><authentication>password-cleartext</authentication></incomingServer><incomingServer type="imap"><hostname>imap.example.com</hostname><port>993</port><socketType>SSL</socketType><username>%EMAILADDRESS%</username><authentication>password-cleartext</authentication></incomingServer><outgoingServer type="smtp"><hostname>smtp.example.com</hostname><port>587</port><socketType>STARTTLS</socketType><username>%EMAILLOCALPART%</username><authentication>password-cleartext</authentication></outgoingServer></emailProvider></clientConfig>"#;

    #[test]
    fn parses_ispdb_and_ignores_pop3() {
        let config = parse_ispdb(XML, "alice@example.com").expect("valid XML");
        assert_eq!(config.imap.host, "imap.example.com");
        assert_eq!(config.imap.username, "alice@example.com");
        assert_eq!(config.smtp.security, SocketSecurity::StartTls);
        assert_eq!(config.smtp.username, "alice");
    }

    #[test]
    fn rejects_plaintext_and_wrong_domain() {
        assert_eq!(
            parse_ispdb(&XML.replace("STARTTLS", "plain"), "alice@example.com"),
            Err(AutoconfigError::NoSecureServers)
        );
        assert_eq!(
            parse_ispdb(XML, "alice@evil.com"),
            Err(AutoconfigError::DomainMismatch)
        );
    }

    #[test]
    fn rejects_unsupported_username_and_malformed_xml() {
        assert_eq!(
            parse_ispdb(
                &XML.replace("%EMAILLOCALPART%", "%UNKNOWN%"),
                "alice@example.com"
            ),
            Err(AutoconfigError::NoSecureServers)
        );
        assert_eq!(
            parse_ispdb("<!DOCTYPE x><clientConfig/>", "alice@example.com"),
            Err(AutoconfigError::MalformedXml)
        );
        assert_eq!(
            parse_ispdb(
                &XML.replace("password-cleartext", "OAuth2"),
                "alice@example.com"
            ),
            Err(AutoconfigError::NoSecureServers)
        );
    }

    #[test]
    fn lookup_order_and_mx_switch() {
        let mut discovery = Discovery::new("a@work.example".into()).expect("valid address");
        assert_eq!(discovery.step, DiscoveryStep::AutoconfigHost);
        discovery.autoconfig_result(None);
        assert_eq!(discovery.step, DiscoveryStep::Ispdb);
        discovery.ispdb_result(None);
        assert_eq!(discovery.step, DiscoveryStep::Mx);
        discovery.mx_result(&["work-example.mail.protection.outlook.com".into()]);
        assert_eq!(discovery.step, DiscoveryStep::OAuth(Provider::Microsoft));
    }

    #[test]
    fn built_in_precedes_network() {
        let mut discovery = Discovery::new("a@gmail.com".into()).expect("valid address");
        assert_eq!(discovery.step, DiscoveryStep::BuiltIn(Provider::Google));
        discovery.accept_builtin();
        assert_eq!(discovery.step, DiscoveryStep::OAuth(Provider::Google));
    }
}
