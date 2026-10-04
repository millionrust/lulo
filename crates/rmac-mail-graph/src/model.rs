//! The documented Microsoft Graph v1.0 JSON shapes this backend reads.
//! Every field is optional where Graph may omit it (delta replies only carry
//! changed properties for some items).

use rmac_mail_storage::{FLAG_ANSWERED, FLAG_DRAFT, FLAG_FLAGGED, FLAG_SEEN};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Page<T> {
    #[serde(default = "Vec::new")]
    pub value: Vec<T>,
    #[serde(rename = "@odata.nextLink")]
    pub next_link: Option<String>,
    #[serde(rename = "@odata.deltaLink")]
    pub delta_link: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: String,
    pub display_name: Option<String>,
    pub child_folder_count: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct EmailAddress {
    pub name: Option<String>,
    pub address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recipient {
    pub email_address: Option<EmailAddress>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowupFlag {
    pub flag_status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub subject: Option<String>,
    pub from: Option<Recipient>,
    pub to_recipients: Option<Vec<Recipient>>,
    pub cc_recipients: Option<Vec<Recipient>>,
    pub received_date_time: Option<String>,
    pub is_read: Option<bool>,
    pub is_draft: Option<bool>,
    pub flag: Option<FollowupFlag>,
    pub body_preview: Option<String>,
    pub internet_message_id: Option<String>,
    #[serde(rename = "@removed")]
    pub removed: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct BatchReply {
    #[serde(default = "Vec::new")]
    pub responses: Vec<BatchResponse>,
}

#[derive(Debug, Deserialize)]
pub struct BatchResponse {
    pub id: String,
    pub status: u16,
    pub body: Option<serde_json::Value>,
}

/// `"Name <address>"` when Graph has a display name, else the address,
/// the form Mail's sidebar and reply code split again.
pub fn mailbox_text(recipient: &Recipient) -> String {
    let Some(email) = &recipient.email_address else {
        return String::new();
    };
    let address = clean(email.address.as_deref().unwrap_or(""));
    match email.name.as_deref().map(clean) {
        Some(name) if !name.is_empty() && name != address => {
            if address.is_empty() {
                name
            } else {
                format!("{name} <{address}>")
            }
        }
        _ => address,
    }
}

pub fn address_list(recipients: Option<&[Recipient]>) -> String {
    recipients
        .into_iter()
        .flatten()
        .filter_map(|recipient| recipient.email_address.as_ref()?.address.as_deref())
        .map(clean)
        .filter(|address| !address.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Server strings never carry control characters into the cache or UI.
pub fn clean(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Local flag bits from a Graph message. Graph has no "answered" property in
/// v1.0, so that bit is kept from the cache. A property the delta reply
/// omitted keeps its cached value too.
pub fn flags(message: &Message, cached: i64) -> i64 {
    let mut bits = cached & FLAG_ANSWERED;
    let keep = |bit: i64| cached & bit;
    bits |= match message.is_read {
        Some(true) => FLAG_SEEN,
        Some(false) => 0,
        None => keep(FLAG_SEEN),
    };
    bits |= match message
        .flag
        .as_ref()
        .and_then(|flag| flag.flag_status.as_deref())
    {
        Some("flagged") => FLAG_FLAGGED,
        Some(_) => 0,
        None => keep(FLAG_FLAGGED),
    };
    bits |= match message.is_draft {
        Some(true) => FLAG_DRAFT,
        Some(false) => 0,
        None => keep(FLAG_DRAFT),
    };
    bits
}

/// Graph timestamps are ISO 8601 in UTC (`2026-10-04T09:12:00Z`).
pub fn unix_time(value: Option<&str>) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value?)
        .ok()
        .map(|time| time.timestamp())
}
