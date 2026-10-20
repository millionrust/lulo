//! Lulo's local contacts: the people Mail knows, which of them are VIPs,
//! and a read-only vCard (3.0 and 4.0) import.
//!
//! Stored as one small JSON document at `~/.local/share/lulo/contacts.json`
//! (`%APPDATA%\Lulo\contacts.json` on Windows), directory 0700, file 0600,
//! atomic writes. Mail reads it for Message ▸ Add Sender to Contacts, the
//! VIPs mailbox, View ▸ Filter ▸ Only from VIP, rules' "Sender is in my
//! contacts / is VIP" and compose's address completion.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;
const FILE_NAME: &str = "contacts.json";
const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
/// A vCard file bigger than this is refused rather than read into memory.
pub const MAX_VCARD_BYTES: usize = 16 * 1024 * 1024;

/// One person.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Contact {
    pub name: String,
    pub emails: Vec<String>,
    pub vip: bool,
}

impl Contact {
    fn has(&self, address: &str) -> bool {
        self.emails
            .iter()
            .any(|email| email.eq_ignore_ascii_case(address))
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Contacts {
    pub contacts: Vec<Contact>,
}

/// The address inside `"Name <address>"`, or `text` itself, trimmed.
pub fn bare_address(text: &str) -> &str {
    text.rsplit_once('<')
        .map_or(text, |(_, rest)| rest.trim_end_matches('>'))
        .trim()
}

impl Contacts {
    /// Whether `address` belongs to any contact.
    pub fn knows(&self, address: &str) -> bool {
        let address = bare_address(address);
        !address.is_empty() && self.contacts.iter().any(|contact| contact.has(address))
    }

    /// Whether `address` belongs to a VIP.
    pub fn is_vip(&self, address: &str) -> bool {
        let address = bare_address(address);
        !address.is_empty()
            && self
                .contacts
                .iter()
                .any(|contact| contact.vip && contact.has(address))
    }

    /// Every contact address, lower-cased.
    pub fn addresses(&self) -> HashSet<String> {
        self.contacts
            .iter()
            .flat_map(|contact| contact.emails.iter())
            .map(|email| email.to_ascii_lowercase())
            .collect()
    }

    /// Every VIP address, lower-cased.
    pub fn vip_addresses(&self) -> HashSet<String> {
        self.contacts
            .iter()
            .filter(|contact| contact.vip)
            .flat_map(|contact| contact.emails.iter())
            .map(|email| email.to_ascii_lowercase())
            .collect()
    }

    /// Message ▸ Add Sender to Contacts. Returns false when the address is
    /// already a contact's (the name is filled in if it was missing).
    pub fn add(&mut self, name: &str, address: &str) -> bool {
        let address = bare_address(address);
        if !valid_address(address) {
            return false;
        }
        if let Some(contact) = self
            .contacts
            .iter_mut()
            .find(|contact| contact.has(address))
        {
            if contact.name.is_empty() && !name.trim().is_empty() {
                contact.name = name.trim().to_owned();
            }
            return false;
        }
        self.contacts.push(Contact {
            name: name.trim().to_owned(),
            emails: vec![address.to_owned()],
            vip: false,
        });
        true
    }

    /// Message ▸ Add Sender to VIPs / Remove from VIPs. Adding a VIP who is
    /// not a contact yet makes them one, as on the Mac.
    pub fn set_vip(&mut self, name: &str, address: &str, vip: bool) -> bool {
        let bare = bare_address(address);
        if !valid_address(bare) {
            return false;
        }
        if vip {
            self.add(name, bare);
        }
        let mut changed = false;
        for contact in &mut self.contacts {
            if contact.has(bare) && contact.vip != vip {
                contact.vip = vip;
                changed = true;
            }
        }
        changed
    }

    /// Address completion candidates: `(name, address)` for every address
    /// whose name or address starts a word with `query`.
    pub fn complete(&self, query: &str) -> Vec<(String, String)> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for contact in &self.contacts {
            let name = contact.name.to_lowercase();
            let name_matches = name.split_whitespace().any(|word| word.starts_with(&query))
                || name.starts_with(&query);
            for email in &contact.emails {
                if name_matches || email.to_lowercase().starts_with(&query) {
                    out.push((contact.name.clone(), email.clone()));
                }
            }
        }
        out
    }

    /// File ▸ Import vCards…: adds every card in `text` with at least one
    /// email address, merging into a contact that already has one of them.
    /// Returns how many cards added or changed a contact.
    pub fn import_vcards(&mut self, text: &str) -> usize {
        let mut changed = 0;
        for card in parse_vcards(text) {
            if card.emails.is_empty() {
                continue;
            }
            let existing = self
                .contacts
                .iter_mut()
                .find(|contact| card.emails.iter().any(|email| contact.has(email)));
            match existing {
                Some(contact) => {
                    let mut touched = false;
                    for email in &card.emails {
                        if !contact.has(email) {
                            contact.emails.push(email.clone());
                            touched = true;
                        }
                    }
                    if contact.name.is_empty() && !card.name.is_empty() {
                        contact.name = card.name.clone();
                        touched = true;
                    }
                    changed += usize::from(touched);
                }
                None => {
                    self.contacts.push(card);
                    changed += 1;
                }
            }
        }
        changed
    }
}

fn valid_address(address: &str) -> bool {
    let Some((local, domain)) = address.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && !address.contains(char::is_whitespace)
        && !address.contains(['<', '>', ',', ';'])
}

/// The cards in a vCard 3.0 or 4.0 document: FN (or N) and every EMAIL.
/// Folded lines (RFC 6350 §3.2) are unfolded, parameters ignored and
/// backslash escapes undone. Anything else on the card is not kept.
pub fn parse_vcards(text: &str) -> Vec<Contact> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = raw.strip_prefix([' ', '\t']) {
            if let Some(last) = lines.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        lines.push(raw.to_owned());
    }
    let mut cards = Vec::new();
    let mut current: Option<(Contact, String)> = None;
    for line in lines {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        // "item1.EMAIL;TYPE=work" → "EMAIL"
        let name = key.split(';').next().unwrap_or_default();
        let name = name
            .rsplit('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        match name.as_str() {
            "BEGIN" if value.trim().eq_ignore_ascii_case("VCARD") => {
                current = Some((Contact::default(), String::new()));
            }
            "END" if value.trim().eq_ignore_ascii_case("VCARD") => {
                if let Some((mut contact, structured)) = current.take() {
                    if contact.name.is_empty() {
                        contact.name = structured;
                    }
                    cards.push(contact);
                }
            }
            "FN" => {
                if let Some((contact, _)) = current.as_mut() {
                    contact.name = unescape(value).trim().to_owned();
                }
            }
            "N" => {
                if let Some((_, structured)) = current.as_mut() {
                    // Family;Given;Additional;Prefix;Suffix → "Given Family"
                    let parts: Vec<String> = value.split(';').map(unescape).collect();
                    let family = parts.first().map(String::as_str).unwrap_or_default();
                    let given = parts.get(1).map(String::as_str).unwrap_or_default();
                    *structured = format!("{given} {family}").trim().to_owned();
                }
            }
            "EMAIL" => {
                if let Some((contact, _)) = current.as_mut() {
                    let email = unescape(value);
                    let email = email.trim().trim_start_matches("mailto:");
                    if valid_address(email) && !contact.has(email) {
                        contact.emails.push(email.to_owned());
                    }
                }
            }
            _ => {}
        }
    }
    cards
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push(' '),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[derive(Deserialize, Serialize)]
struct Stored {
    version: u32,
    #[serde(flatten)]
    contacts: Contacts,
}

/// Where the store lives.
pub fn store_path() -> PathBuf {
    #[cfg(windows)]
    if let Some(appdata) = std::env::var_os("APPDATA").map(PathBuf::from) {
        return appdata.join("Lulo").join(FILE_NAME);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let data = match std::env::var_os("XDG_DATA_HOME").map(PathBuf::from) {
        Some(path) if path.is_absolute() => path,
        _ => home.join(".local").join("share"),
    };
    data.join("lulo").join(FILE_NAME)
}

/// The saved contacts, or none if nothing was saved yet. Blocking: call it
/// off the UI thread.
pub fn load() -> io::Result<Contacts> {
    load_from(&store_path())
}

/// Saves `contacts` atomically. Blocking: call it off the UI thread.
pub fn save(contacts: &Contacts) -> io::Result<()> {
    save_to(&store_path(), contacts)
}

pub fn load_from(path: &Path) -> io::Result<Contacts> {
    let bytes = match rmac_storage::read_bounded_no_follow(path, MAX_FILE_BYTES) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Contacts::default()),
        Err(error) => return Err(error),
    };
    let stored: Stored = serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if stored.version != VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("contacts.json version {} is not supported", stored.version),
        ));
    }
    Ok(stored.contacts)
}

pub fn save_to(path: &Path, contacts: &Contacts) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        rmac_storage::create_dir_all_private(parent)?;
    }
    let stored = Stored {
        version: VERSION,
        contacts: contacts.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&stored)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    rmac_storage::atomic_write_private(path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARDS: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nN:Ortiz;Sam;;;\r\nFN:Sam Ortiz\r\nEMAIL;TYPE=INTERNET;TYPE=WORK:sam@example.test\r\nitem1.EMAIL;type=INTERNET:sam.o\r\n rtiz@example.test\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:4.0\r\nN:Lee;Anna;;;\r\nEMAIL;PREF=1:mailto:anna@example.test\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:4.0\r\nFN:No Mail\r\nEND:VCARD\r\n";

    #[test]
    fn parses_vcard_three_and_four_with_folding_and_structured_names() {
        let cards = parse_vcards(CARDS);
        assert_eq!(cards.len(), 3);
        assert_eq!(cards[0].name, "Sam Ortiz");
        assert_eq!(
            cards[0].emails,
            vec!["sam@example.test", "sam.ortiz@example.test"]
        );
        assert_eq!(cards[1].name, "Anna Lee");
        assert_eq!(cards[1].emails, vec!["anna@example.test"]);
        assert!(cards[2].emails.is_empty());
    }

    #[test]
    fn import_skips_cards_without_mail_and_merges_known_people() {
        let mut contacts = Contacts::default();
        contacts.add("", "SAM@example.test");
        assert_eq!(contacts.import_vcards(CARDS), 2);
        assert_eq!(contacts.contacts.len(), 2);
        assert_eq!(contacts.contacts[0].name, "Sam Ortiz");
        assert_eq!(contacts.contacts[0].emails.len(), 2);
        // A second import changes nothing.
        assert_eq!(contacts.import_vcards(CARDS), 0);
    }

    #[test]
    fn vips_are_contacts_and_matching_ignores_case_and_display_names() {
        let mut contacts = Contacts::default();
        assert!(contacts.set_vip("Anna Lee", "Anna Lee <anna@example.test>", true));
        assert!(contacts.knows("ANNA@example.test"));
        assert!(contacts.is_vip("Anna <anna@EXAMPLE.test>"));
        assert!(!contacts.is_vip("sam@example.test"));
        assert!(contacts.vip_addresses().contains("anna@example.test"));
        assert!(contacts.set_vip("", "anna@example.test", false));
        assert!(!contacts.is_vip("anna@example.test"));
        assert!(contacts.knows("anna@example.test"));
        assert!(!contacts.add("x", "not an address"));
    }

    #[test]
    fn completion_matches_name_words_and_address_starts() {
        let mut contacts = Contacts::default();
        contacts.import_vcards(CARDS);
        let found = contacts.complete("ort");
        assert_eq!(found.len(), 2);
        assert_eq!(contacts.complete("anna").len(), 1);
        assert!(contacts.complete("zzz").is_empty());
    }

    #[test]
    fn round_trips_through_the_store_file() {
        let dir = std::env::temp_dir().join(format!("rmac-contacts-test-{}", std::process::id()));
        let path = dir.join("contacts.json");
        let mut contacts = Contacts::default();
        contacts.set_vip("Anna", "anna@example.test", true);
        save_to(&path, &contacts).unwrap();
        assert_eq!(load_from(&path).unwrap(), contacts);
        assert_eq!(
            load_from(&dir.join("missing.json")).unwrap(),
            Contacts::default()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
