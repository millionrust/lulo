//! Bounded HTTPS autoconfig and system-resolver MX lookup.

use std::io::Read;
use std::time::Duration;

use rmac_accounts::autoconfig::{parse_ispdb, Discovery, DiscoveryStep, MailConfig, MailServer};
use rmac_accounts::{model::email_domain, provider::Provider};

use crate::Error;

const MAX_XML: u64 = 256 * 1024;

/// Discovery is a blocking worker operation. HTTP and DNS are separate to make
/// ordering, TLS policy, and failure fallback testable without the network.
pub trait DiscoveryNetwork {
    fn get_xml(&self, url: &str) -> Result<String, Error>;
    fn mx_hosts(&self, domain: &str) -> Result<Vec<String>, Error>;
}

pub struct SystemDiscovery;

impl DiscoveryNetwork for SystemDiscovery {
    fn get_xml(&self, url: &str) -> Result<String, Error> {
        if !url.starts_with("https://") {
            return Err(Error::InvalidResponse);
        }
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(8))
            .redirects(0)
            .build();
        let response = agent.get(url).call().map_err(|_| Error::Network)?;
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(MAX_XML + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Network)?;
        if bytes.len() as u64 > MAX_XML {
            return Err(Error::InvalidResponse);
        }
        String::from_utf8(bytes).map_err(|_| Error::InvalidResponse)
    }

    fn mx_hosts(&self, domain: &str) -> Result<Vec<String>, Error> {
        mx_hosts(domain)
    }
}

pub fn discover(network: &impl DiscoveryNetwork, address: &str) -> Result<Discovery, Error> {
    let mut discovery = Discovery::new(address.to_owned()).map_err(|_| Error::InvalidResponse)?;
    let domain = email_domain(address).ok_or(Error::InvalidResponse)?;
    if let DiscoveryStep::BuiltIn(provider) = discovery.step {
        if let Some(preset) = provider.info().servers {
            let server = |host: &str, port: u16, security| MailServer {
                host: host.to_owned(),
                port,
                security,
                username: address.to_owned(),
            };
            discovery.config = Some(MailConfig {
                imap: server(preset.imap_host, preset.imap_port, preset.imap_security),
                smtp: server(preset.smtp_host, preset.smtp_port, preset.smtp_security),
            });
        }
        discovery.accept_builtin();
        return Ok(discovery);
    }
    let autoconfig = format!("https://autoconfig.{domain}/mail/config-v1.1.xml");
    discovery.autoconfig_result(fetch_config(network, &autoconfig, address));
    if discovery.step == DiscoveryStep::Ispdb {
        let ispdb = format!("https://autoconfig.thunderbird.net/v1.1/{domain}");
        discovery.ispdb_result(fetch_config(network, &ispdb, address));
    }
    if discovery.step == DiscoveryStep::Mx {
        let hosts = network.mx_hosts(domain).unwrap_or_default();
        let provider = hosts.first().and_then(|host| Provider::from_mx_host(host));
        if provider.is_some()
            && hosts
                .iter()
                .all(|host| Provider::from_mx_host(host) == provider)
        {
            discovery.mx_result(&hosts);
        } else {
            discovery.mx_result(&[]);
        }
    }
    Ok(discovery)
}

fn fetch_config(network: &impl DiscoveryNetwork, url: &str, address: &str) -> Option<MailConfig> {
    let xml = network.get_xml(url).ok()?;
    parse_ispdb(&xml, address).ok()
}

#[cfg(target_os = "linux")]
fn mx_hosts(domain: &str) -> Result<Vec<String>, Error> {
    use std::ffi::CString;

    // libc's resolver honours /etc/resolv.conf and the user's NSS setup.
    #[link(name = "resolv")]
    unsafe extern "C" {
        fn res_query(
            name: *const libc::c_char,
            class: libc::c_int,
            kind: libc::c_int,
            answer: *mut libc::c_uchar,
            answer_len: libc::c_int,
        ) -> libc::c_int;
    }
    let name = CString::new(domain).map_err(|_| Error::InvalidResponse)?;
    let mut answer = [0_u8; 4096];
    let size = unsafe {
        res_query(
            name.as_ptr(),
            1,
            15,
            answer.as_mut_ptr(),
            answer.len() as i32,
        )
    };
    if size < 0 {
        return Err(Error::Network);
    }
    if size as usize > answer.len() {
        return Err(Error::InvalidResponse);
    }
    parse_mx_response(&answer[..size as usize])
}

#[cfg(not(target_os = "linux"))]
fn mx_hosts(_domain: &str) -> Result<Vec<String>, Error> {
    Err(Error::Unavailable)
}

/// Only DNS answer names are used to select an OAuth provider; malformed
/// packets fail closed to the manual server form.
#[cfg(any(target_os = "linux", test))]
fn parse_mx_response(packet: &[u8]) -> Result<Vec<String>, Error> {
    if packet.len() < 12 || packet[3] & 0x0f != 0 {
        return Err(Error::InvalidResponse);
    }
    let questions = u16::from_be_bytes([packet[4], packet[5]]) as usize;
    let answers = u16::from_be_bytes([packet[6], packet[7]]) as usize;
    let mut offset = 12;
    for _ in 0..questions {
        let _ = read_name(packet, &mut offset)?;
        offset = offset.checked_add(4).ok_or(Error::InvalidResponse)?;
        if offset > packet.len() {
            return Err(Error::InvalidResponse);
        }
    }
    let mut hosts = Vec::new();
    for _ in 0..answers {
        let _ = read_name(packet, &mut offset)?;
        if offset + 10 > packet.len() {
            return Err(Error::InvalidResponse);
        }
        let kind = u16::from_be_bytes([packet[offset], packet[offset + 1]]);
        let class = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]);
        let length = u16::from_be_bytes([packet[offset + 8], packet[offset + 9]]) as usize;
        offset += 10;
        let end = offset.checked_add(length).ok_or(Error::InvalidResponse)?;
        if end > packet.len() {
            return Err(Error::InvalidResponse);
        }
        if kind == 15 && class == 1 && length >= 3 {
            let mut name_offset = offset + 2;
            let name = read_name(packet, &mut name_offset)?;
            if name_offset == end {
                hosts.push(name);
            }
        }
        offset = end;
    }
    Ok(hosts)
}

#[cfg(any(target_os = "linux", test))]
fn read_name(packet: &[u8], offset: &mut usize) -> Result<String, Error> {
    let mut cursor = *offset;
    let mut jumped = false;
    let mut labels = Vec::new();
    for _ in 0..32 {
        let len = *packet.get(cursor).ok_or(Error::InvalidResponse)?;
        if len & 0xc0 == 0xc0 {
            let low = *packet.get(cursor + 1).ok_or(Error::InvalidResponse)?;
            let target = ((usize::from(len & 0x3f)) << 8) | usize::from(low);
            if !jumped {
                *offset = cursor + 2;
                jumped = true;
            }
            if target >= cursor {
                return Err(Error::InvalidResponse);
            }
            cursor = target;
        } else if len == 0 {
            if !jumped {
                *offset = cursor + 1;
            }
            return Ok(labels.join("."));
        } else if len > 63 {
            return Err(Error::InvalidResponse);
        } else {
            let start = cursor + 1;
            let end = start + usize::from(len);
            let label = packet.get(start..end).ok_or(Error::InvalidResponse)?;
            if label.first() == Some(&b'-')
                || label.last() == Some(&b'-')
                || !label
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
            {
                return Err(Error::InvalidResponse);
            }
            labels.push(
                std::str::from_utf8(label)
                    .map_err(|_| Error::InvalidResponse)?
                    .to_ascii_lowercase(),
            );
            cursor = end;
        }
    }
    Err(Error::InvalidResponse)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_accounts::{autoconfig::DiscoveryStep, provider::Provider};

    struct Fake;
    impl DiscoveryNetwork for Fake {
        fn get_xml(&self, _url: &str) -> Result<String, Error> {
            Err(Error::Network)
        }
        fn mx_hosts(&self, _domain: &str) -> Result<Vec<String>, Error> {
            Ok(vec!["tenant.mail.protection.outlook.com".into()])
        }
    }

    #[test]
    fn falls_back_to_mx_after_both_https_sources() {
        let result = discover(&Fake, "a@company.example").unwrap();
        assert_eq!(result.step, DiscoveryStep::OAuth(Provider::Microsoft));
    }

    #[test]
    fn built_in_provider_avoids_network() {
        let result = discover(&Fake, "a@gmail.com").unwrap();
        assert_eq!(result.step, DiscoveryStep::OAuth(Provider::Google));
        assert_eq!(result.config.unwrap().imap.host, "imap.gmail.com");
    }

    #[test]
    fn icloud_has_secure_password_mail_servers() {
        let result = discover(&Fake, "a@icloud.com").unwrap();
        assert_eq!(result.step, DiscoveryStep::Configured);
        let config = result.config.unwrap();
        assert_eq!(config.imap.host, "imap.mail.me.com");
        assert_eq!(config.smtp.username, "a@icloud.com");
    }

    #[test]
    fn parses_mx_and_rejects_compression_loops() {
        let mut packet = vec![0, 0, 0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0];
        packet.extend_from_slice(b"\x07example\x03com\x00\x00\x0f\x00\x01");
        packet.extend_from_slice(&[0xc0, 0x0c, 0, 15, 0, 1, 0, 0, 0, 60]);
        let target = b"\x04mail\x0aprotection\x07outlook\x03com\x00";
        packet.extend_from_slice(&((target.len() + 2) as u16).to_be_bytes());
        packet.extend_from_slice(&[0, 10]);
        packet.extend_from_slice(target);
        assert_eq!(
            parse_mx_response(&packet).unwrap(),
            vec!["mail.protection.outlook.com"]
        );
        packet[12] = 0xc0;
        packet[13] = 0x0c;
        assert_eq!(parse_mx_response(&packet), Err(Error::InvalidResponse));
    }

    #[test]
    fn mixed_mx_hosts_do_not_choose_oauth_provider() {
        struct Mixed;
        impl DiscoveryNetwork for Mixed {
            fn get_xml(&self, _url: &str) -> Result<String, Error> {
                Err(Error::Network)
            }
            fn mx_hosts(&self, _domain: &str) -> Result<Vec<String>, Error> {
                Ok(vec![
                    "mail.example.net".into(),
                    "tenant.mail.protection.outlook.com".into(),
                ])
            }
        }
        assert_eq!(
            discover(&Mixed, "a@company.example").unwrap().step,
            DiscoveryStep::Manual
        );
    }
}
