//! A minimal IPP/1.1 (RFC 8010/8011) encoder and decoder: just what the
//! read-only CUPS queries need. Unknown value tags are kept as raw bytes and
//! collections are skipped, so a newer CUPS cannot break parsing. Every
//! length is bounds-checked; a malformed response is an error, never a
//! panic.

use std::collections::BTreeMap;

pub const GET_JOBS: u16 = 0x000A;
pub const CUPS_GET_DEFAULT: u16 = 0x4001;
pub const CUPS_GET_PRINTERS: u16 = 0x4002;

const OPERATION_GROUP: u8 = 0x01;
const END_OF_ATTRIBUTES: u8 = 0x03;

pub const TAG_INTEGER: u8 = 0x21;
pub const TAG_BOOLEAN: u8 = 0x22;
pub const TAG_ENUM: u8 = 0x23;
pub const TAG_TEXT_WITH_LANGUAGE: u8 = 0x35;
pub const TAG_NAME_WITH_LANGUAGE: u8 = 0x36;
const TAG_BEGIN_COLLECTION: u8 = 0x34;
const TAG_END_COLLECTION: u8 = 0x37;
pub const TAG_TEXT: u8 = 0x41;
pub const TAG_NAME: u8 = 0x42;
pub const TAG_KEYWORD: u8 = 0x44;
pub const TAG_URI: u8 = 0x45;
pub const TAG_CHARSET: u8 = 0x47;
pub const TAG_LANGUAGE: u8 = 0x48;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Integer(i32),
    Boolean(bool),
    Text(String),
    Other(u8, Vec<u8>),
}

impl Value {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_integer(&self) -> Option<i32> {
        match self {
            Value::Integer(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Boolean(value) => Some(*value),
            _ => None,
        }
    }
}

/// One attribute group (a printer, a job): name → values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Group {
    pub tag: u8,
    pub attributes: BTreeMap<String, Vec<Value>>,
}

impl Group {
    pub fn text(&self, name: &str) -> Option<&str> {
        self.attributes.get(name)?.first()?.as_text()
    }

    pub fn texts(&self, name: &str) -> Vec<&str> {
        self.attributes
            .get(name)
            .map(|values| values.iter().filter_map(Value::as_text).collect())
            .unwrap_or_default()
    }

    pub fn integer(&self, name: &str) -> Option<i32> {
        self.attributes.get(name)?.first()?.as_integer()
    }

    pub fn boolean(&self, name: &str) -> Option<bool> {
        self.attributes.get(name)?.first()?.as_bool()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Response {
    pub status: u16,
    pub groups: Vec<Group>,
}

impl Response {
    /// successful-ok … successful-ok-events-complete (0x0000–0x00FF).
    pub fn is_success(&self) -> bool {
        self.status < 0x0100
    }

    pub fn groups_tagged(&self, tag: u8) -> impl Iterator<Item = &Group> {
        self.groups.iter().filter(move |group| group.tag == tag)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Malformed;

/// An IPP request under construction (operation attributes only).
pub struct Request {
    bytes: Vec<u8>,
}

impl Request {
    pub fn new(operation: u16, request_id: u32) -> Self {
        let mut bytes = vec![2, 0];
        bytes.extend_from_slice(&operation.to_be_bytes());
        bytes.extend_from_slice(&request_id.to_be_bytes());
        bytes.push(OPERATION_GROUP);
        let mut request = Self { bytes };
        request.attribute(TAG_CHARSET, "attributes-charset", "utf-8");
        request.attribute(TAG_LANGUAGE, "attributes-natural-language", "en");
        request
    }

    /// Add one single-valued attribute. Names and values longer than IPP's
    /// 16-bit lengths are truncated to stay well-formed.
    pub fn attribute(&mut self, tag: u8, name: &str, value: &str) -> &mut Self {
        self.push(tag, name.as_bytes(), value.as_bytes());
        self
    }

    pub fn integer(&mut self, tag: u8, name: &str, value: i32) -> &mut Self {
        self.push(tag, name.as_bytes(), &value.to_be_bytes());
        self
    }

    /// A multi-valued keyword attribute (`requested-attributes`).
    pub fn keywords(&mut self, name: &str, values: &[&str]) -> &mut Self {
        for (index, value) in values.iter().enumerate() {
            let name = if index == 0 { name.as_bytes() } else { b"" };
            self.push(TAG_KEYWORD, name, value.as_bytes());
        }
        self
    }

    fn push(&mut self, tag: u8, name: &[u8], value: &[u8]) {
        let name = &name[..name.len().min(u16::MAX as usize)];
        let value = &value[..value.len().min(u16::MAX as usize)];
        self.bytes.push(tag);
        self.bytes
            .extend_from_slice(&(name.len() as u16).to_be_bytes());
        self.bytes.extend_from_slice(name);
        self.bytes
            .extend_from_slice(&(value.len() as u16).to_be_bytes());
        self.bytes.extend_from_slice(value);
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.bytes.push(END_OF_ATTRIBUTES);
        self.bytes
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], Malformed> {
        let end = self.at.checked_add(count).ok_or(Malformed)?;
        let slice = self.bytes.get(self.at..end).ok_or(Malformed)?;
        self.at = end;
        Ok(slice)
    }

    fn byte(&mut self) -> Result<u8, Malformed> {
        Ok(self.take(1)?[0])
    }

    fn short(&mut self) -> Result<u16, Malformed> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }
}

fn decode_value(tag: u8, raw: &[u8]) -> Result<Value, Malformed> {
    Ok(match tag {
        TAG_INTEGER | TAG_ENUM => {
            let bytes: [u8; 4] = raw.try_into().map_err(|_| Malformed)?;
            Value::Integer(i32::from_be_bytes(bytes))
        }
        TAG_BOOLEAN => match raw {
            [value] => Value::Boolean(*value != 0),
            _ => return Err(Malformed),
        },
        TAG_TEXT_WITH_LANGUAGE | TAG_NAME_WITH_LANGUAGE => {
            let mut cursor = Cursor { bytes: raw, at: 0 };
            let language = cursor.short()? as usize;
            cursor.take(language)?;
            let text = cursor.short()? as usize;
            Value::Text(String::from_utf8_lossy(cursor.take(text)?).into_owned())
        }
        0x40..=0x4F => Value::Text(String::from_utf8_lossy(raw).into_owned()),
        _ => Value::Other(tag, raw.to_vec()),
    })
}

/// Decode a complete IPP response body.
pub fn parse(bytes: &[u8]) -> Result<Response, Malformed> {
    let mut cursor = Cursor { bytes, at: 0 };
    let _version = cursor.take(2)?;
    let status = cursor.short()?;
    let _request_id = cursor.take(4)?;
    let mut groups: Vec<Group> = Vec::new();
    let mut last_name: Option<String> = None;
    let mut collection_depth = 0_usize;
    loop {
        let tag = cursor.byte()?;
        if tag == END_OF_ATTRIBUTES && collection_depth == 0 {
            break;
        }
        if tag < 0x10 {
            if collection_depth != 0 {
                return Err(Malformed);
            }
            groups.push(Group {
                tag,
                attributes: BTreeMap::new(),
            });
            last_name = None;
            continue;
        }
        let name_len = cursor.short()? as usize;
        let name = cursor.take(name_len)?;
        let value_len = cursor.short()? as usize;
        let raw = cursor.take(value_len)?;
        if tag == TAG_BEGIN_COLLECTION {
            collection_depth += 1;
            continue;
        }
        if tag == TAG_END_COLLECTION {
            collection_depth = collection_depth.checked_sub(1).ok_or(Malformed)?;
            continue;
        }
        if collection_depth > 0 {
            continue;
        }
        let group = groups.last_mut().ok_or(Malformed)?;
        let name = if name.is_empty() {
            last_name.clone().ok_or(Malformed)?
        } else {
            let name = String::from_utf8_lossy(name).into_owned();
            last_name = Some(name.clone());
            name
        };
        // Out-of-band values (no-value, unknown) carry nothing useful.
        if (0x10..0x20).contains(&tag) {
            group.attributes.entry(name).or_default();
            continue;
        }
        group
            .attributes
            .entry(name)
            .or_default()
            .push(decode_value(tag, raw)?);
    }
    Ok(Response { status, groups })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Build a response body the way cupsd lays one out.
    pub(crate) struct ResponseBuilder {
        bytes: Vec<u8>,
    }

    impl ResponseBuilder {
        pub(crate) fn new(status: u16) -> Self {
            let mut bytes = vec![2, 0];
            bytes.extend_from_slice(&status.to_be_bytes());
            bytes.extend_from_slice(&1_u32.to_be_bytes());
            let mut builder = Self { bytes };
            builder.group(OPERATION_GROUP);
            builder.raw(TAG_CHARSET, "attributes-charset", b"utf-8");
            builder
        }

        pub(crate) fn group(&mut self, tag: u8) -> &mut Self {
            self.bytes.push(tag);
            self
        }

        pub(crate) fn raw(&mut self, tag: u8, name: &str, value: &[u8]) -> &mut Self {
            self.bytes.push(tag);
            self.bytes
                .extend_from_slice(&(name.len() as u16).to_be_bytes());
            self.bytes.extend_from_slice(name.as_bytes());
            self.bytes
                .extend_from_slice(&(value.len() as u16).to_be_bytes());
            self.bytes.extend_from_slice(value);
            self
        }

        pub(crate) fn text(&mut self, tag: u8, name: &str, value: &str) -> &mut Self {
            self.raw(tag, name, value.as_bytes())
        }

        pub(crate) fn int(&mut self, tag: u8, name: &str, value: i32) -> &mut Self {
            self.raw(tag, name, &value.to_be_bytes())
        }

        pub(crate) fn finish(&mut self) -> Vec<u8> {
            let mut bytes = self.bytes.clone();
            bytes.push(END_OF_ATTRIBUTES);
            bytes
        }
    }

    #[test]
    fn requests_encode_the_standard_operation_attributes() {
        let mut request = Request::new(CUPS_GET_PRINTERS, 7);
        request.keywords("requested-attributes", &["printer-name", "printer-state"]);
        let bytes = request.finish();
        assert_eq!(&bytes[..8], &[2, 0, 0x40, 0x02, 0, 0, 0, 7]);
        assert_eq!(bytes[8], OPERATION_GROUP);
        assert_eq!(*bytes.last().unwrap(), END_OF_ATTRIBUTES);
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("attributes-charset"));
        assert!(text.contains("utf-8"));
        // The second keyword is an additional value: empty name.
        let second = bytes
            .windows(2 + 2 + "printer-state".len())
            .position(|window| window.ends_with(b"printer-state"))
            .unwrap();
        assert_eq!(&bytes[second - 1..second + 2], &[TAG_KEYWORD, 0, 0]);
    }

    #[test]
    fn responses_decode_groups_multi_values_and_languages() {
        let mut language_text = Vec::new();
        language_text.extend_from_slice(&2_u16.to_be_bytes());
        language_text.extend_from_slice(b"en");
        language_text.extend_from_slice(&5_u16.to_be_bytes());
        language_text.extend_from_slice(b"Hallo");
        let body = ResponseBuilder::new(0)
            .group(0x04)
            .text(TAG_NAME, "printer-name", "Office")
            .int(TAG_ENUM, "printer-state", 3)
            .text(TAG_KEYWORD, "printer-state-reasons", "none")
            .text(TAG_KEYWORD, "", "offline-report")
            .raw(TAG_TEXT_WITH_LANGUAGE, "printer-info", &language_text)
            .raw(TAG_BOOLEAN, "printer-is-shared", &[1])
            .raw(0x13, "printer-location", b"")
            .group(0x04)
            .text(TAG_NAME, "printer-name", "Home")
            .finish();
        let response = parse(&body).unwrap();
        assert!(response.is_success());
        let printers: Vec<_> = response.groups_tagged(0x04).collect();
        assert_eq!(printers.len(), 2);
        assert_eq!(printers[0].text("printer-name"), Some("Office"));
        assert_eq!(printers[0].integer("printer-state"), Some(3));
        assert_eq!(
            printers[0].texts("printer-state-reasons"),
            ["none", "offline-report"]
        );
        assert_eq!(printers[0].text("printer-info"), Some("Hallo"));
        assert_eq!(printers[0].boolean("printer-is-shared"), Some(true));
        assert_eq!(printers[0].text("printer-location"), None);
        assert_eq!(printers[1].text("printer-name"), Some("Home"));
    }

    #[test]
    fn collections_are_skipped() {
        let body = ResponseBuilder::new(0)
            .group(0x04)
            .raw(TAG_BEGIN_COLLECTION, "media-col-default", b"")
            .text(0x4A, "", "media-size")
            .raw(TAG_BEGIN_COLLECTION, "", b"")
            .text(0x4A, "", "x-dimension")
            .int(TAG_INTEGER, "", 21000)
            .raw(TAG_END_COLLECTION, "", b"")
            .raw(TAG_END_COLLECTION, "", b"")
            .text(TAG_NAME, "printer-name", "Office")
            .finish();
        let response = parse(&body).unwrap();
        let printer = response.groups_tagged(0x04).next().unwrap();
        assert_eq!(printer.text("printer-name"), Some("Office"));
        assert!(!printer.attributes.contains_key("media-col-default"));
    }

    #[test]
    fn malformed_bodies_are_errors_not_panics() {
        let good = ResponseBuilder::new(0)
            .group(0x04)
            .text(TAG_NAME, "printer-name", "Office")
            .finish();
        for cut in 0..good.len() {
            assert_eq!(parse(&good[..cut]), Err(Malformed), "cut at {cut}");
        }
        // An additional value with no attribute before it.
        let orphan = ResponseBuilder::new(0)
            .group(0x04)
            .text(TAG_NAME, "", "x")
            .finish();
        assert_eq!(parse(&orphan), Err(Malformed));
        // A wrong-size integer.
        let short_int = ResponseBuilder::new(0)
            .group(0x04)
            .raw(TAG_INTEGER, "job-id", &[1, 2])
            .finish();
        assert_eq!(parse(&short_int), Err(Malformed));
    }

    #[test]
    fn error_statuses_are_not_success() {
        let body = ResponseBuilder::new(0x0406).finish();
        assert!(!parse(&body).unwrap().is_success());
    }
}
