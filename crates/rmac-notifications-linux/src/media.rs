//! Frozen, bounded media accepted from the XDG Notification portal.

use std::fmt;
use std::io::Cursor;
use std::sync::Arc;

use image::{GenericImageView as _, ImageError, ImageFormat, ImageReader};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use zbus::zvariant::{OwnedValue, Value};

pub const MAX_ICON_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SVG_BYTES: usize = 4_096;
pub const MAX_ICON_EDGE: u32 = 512;
pub const MAX_THEMED_NAMES: usize = 16;
pub const MAX_ICON_NAME_BYTES: usize = 128;
pub const MAX_SOUND_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_SOUND_SECONDS: u64 = 15;
const MAX_SVG_ELEMENTS: usize = 4_096;
const MAX_SVG_ATTRIBUTES: usize = 16_384;
const MAX_SVG_DEPTH: usize = 64;
const MAX_OGG_PAGES: usize = 65_536;
const MAX_OGG_PACKET_PREFIX: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    WrongType,
    Unsupported,
    Unsealed,
    Empty,
    TooLarge,
    Malformed,
    UnsafeSvg,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Error {
    pub field: &'static str,
    pub kind: ErrorKind,
}

impl Error {
    fn new(field: &'static str, kind: ErrorKind) -> Self {
        Self { field, kind }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid notification media field {} ({:?})",
            self.field, self.kind
        )
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IconFormat {
    Png,
    Jpeg,
    Svg,
}

#[derive(Clone, Eq, PartialEq)]
pub enum Icon {
    Themed(Vec<String>),
    File {
        format: IconFormat,
        bytes: Arc<[u8]>,
    },
}

impl fmt::Debug for Icon {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Themed(names) => formatter
                .debug_struct("Themed")
                .field("names", &format_args!("<{} redacted>", names.len()))
                .finish(),
            Self::File { format, bytes } => formatter
                .debug_struct("File")
                .field("format", format)
                .field("bytes", &bytes.len())
                .finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SoundFormat {
    OggOpus,
    OggVorbis,
    WavPcm,
}

#[derive(Clone, Eq, PartialEq)]
pub struct CustomSound {
    pub format: SoundFormat,
    bytes: Arc<[u8]>,
}

impl CustomSound {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for CustomSound {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CustomSound")
            .field("format", &self.format)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NotificationMedia {
    pub icon: Option<Icon>,
    pub sound: Option<CustomSound>,
}

impl NotificationMedia {
    pub(crate) fn retain_for(&mut self, delivery: rmac_notifications::Delivery) {
        if !delivery.banner {
            self.icon = None;
        }
        if !delivery.sound {
            self.sound = None;
        }
    }
}

pub fn take_icon(value: OwnedValue) -> Result<Option<Icon>, Error> {
    if let Ok(name) = <&str>::try_from(&value) {
        return Ok(Some(Icon::Themed(validate_icon_names(vec![
            name.to_owned()
        ])?)));
    }
    let (kind, payload): (String, OwnedValue) = value
        .try_into()
        .map_err(|_| Error::new("icon", ErrorKind::WrongType))?;
    match kind.as_str() {
        "themed" => {
            let names = Value::from(payload)
                .downcast::<Vec<String>>()
                .map_err(|_| Error::new("icon", ErrorKind::WrongType))?;
            Ok(Some(Icon::Themed(validate_icon_names(names)?)))
        }
        "file-descriptor" => {
            let bytes = read_descriptor(payload, "icon", MAX_ICON_BYTES)?;
            let format = validate_icon_bytes(&bytes)?;
            Ok(Some(Icon::File {
                format,
                bytes: bytes.into(),
            }))
        }
        // Version 2 backends must never receive deprecated byte icons. Unknown
        // future kinds are optional presentation data and remain ignorable.
        "bytes" => Err(Error::new("icon", ErrorKind::Unsupported)),
        _ => Ok(None),
    }
}

pub fn take_sound(value: OwnedValue) -> Result<SoundValue, Error> {
    if let Ok(name) = <&str>::try_from(&value) {
        return match name {
            "default" => Ok(SoundValue::Default),
            "silent" => Ok(SoundValue::Silent),
            _ => Err(Error::new("sound", ErrorKind::Unsupported)),
        };
    }
    let (kind, payload): (String, OwnedValue) = value
        .try_into()
        .map_err(|_| Error::new("sound", ErrorKind::WrongType))?;
    if kind != "file-descriptor" {
        return Ok(SoundValue::Unspecified);
    }
    let bytes = read_descriptor(payload, "sound", MAX_SOUND_BYTES)?;
    let format = validate_sound_bytes(&bytes)?;
    Ok(SoundValue::Custom(CustomSound {
        format,
        bytes: bytes.into(),
    }))
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum SoundValue {
    #[default]
    Unspecified,
    Default,
    Silent,
    Custom(CustomSound),
}

fn validate_icon_names(names: Vec<String>) -> Result<Vec<String>, Error> {
    if names.is_empty() || names.len() > MAX_THEMED_NAMES {
        return Err(Error::new("icon", ErrorKind::TooLarge));
    }
    if names.iter().any(|name| {
        name.is_empty()
            || name.len() > MAX_ICON_NAME_BYTES
            || name.starts_with('.')
            || name.contains('/')
            || name.contains('\\')
            || !name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
    }) {
        return Err(Error::new("icon", ErrorKind::Malformed));
    }
    Ok(names)
}

pub fn validate_icon_bytes(bytes: &[u8]) -> Result<IconFormat, Error> {
    if bytes.is_empty() {
        return Err(Error::new("icon", ErrorKind::Empty));
    }
    if bytes.len() > MAX_ICON_BYTES {
        return Err(Error::new("icon", ErrorKind::TooLarge));
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        validate_raster(bytes, ImageFormat::Png)?;
        return Ok(IconFormat::Png);
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        validate_raster(bytes, ImageFormat::Jpeg)?;
        return Ok(IconFormat::Jpeg);
    }
    validate_svg(bytes)?;
    Ok(IconFormat::Svg)
}

fn validate_raster(bytes: &[u8], format: ImageFormat) -> Result<(), Error> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_ICON_EDGE);
    limits.max_image_height = Some(MAX_ICON_EDGE);
    limits.max_alloc = Some(u64::from(MAX_ICON_EDGE) * u64::from(MAX_ICON_EDGE) * 8);
    reader.limits(limits);
    let decoded = reader.decode().map_err(map_image_error)?;
    let (width, height) = decoded.dimensions();
    if width == 0 || width != height || width > MAX_ICON_EDGE {
        return Err(Error::new("icon", ErrorKind::Malformed));
    }
    Ok(())
}

fn map_image_error(error: ImageError) -> Error {
    match error {
        ImageError::Limits(_) => Error::new("icon", ErrorKind::TooLarge),
        _ => Error::new("icon", ErrorKind::Malformed),
    }
}

fn validate_svg(bytes: &[u8]) -> Result<(), Error> {
    if bytes.len() > MAX_SVG_BYTES {
        return Err(Error::new("icon", ErrorKind::TooLarge));
    }
    let text =
        std::str::from_utf8(bytes).map_err(|_| Error::new("icon", ErrorKind::Unsupported))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut reader = Reader::from_str(text);
    reader.config_mut().check_end_names = true;
    let mut elements = 0_usize;
    let mut attributes = 0_usize;
    let mut depth = 0_usize;
    let mut root_seen = false;
    let mut root_closed = false;
    let mut square = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => {
                if root_closed {
                    return Err(Error::new("icon", ErrorKind::Malformed));
                }
                inspect_svg(
                    &start,
                    &mut elements,
                    &mut attributes,
                    &mut root_seen,
                    &mut square,
                )?;
                depth = depth.saturating_add(1);
                if depth > MAX_SVG_DEPTH {
                    return Err(Error::new("icon", ErrorKind::TooLarge));
                }
            }
            Ok(Event::Empty(start)) => {
                if root_closed {
                    return Err(Error::new("icon", ErrorKind::Malformed));
                }
                let is_root = !root_seen;
                inspect_svg(
                    &start,
                    &mut elements,
                    &mut attributes,
                    &mut root_seen,
                    &mut square,
                )?;
                root_closed |= is_root;
            }
            Ok(Event::End(_)) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| Error::new("icon", ErrorKind::Malformed))?;
                root_closed |= depth == 0;
            }
            Ok(Event::DocType(_) | Event::PI(_)) => {
                return Err(Error::new("icon", ErrorKind::UnsafeSvg));
            }
            Ok(Event::Text(text))
                if (!root_seen || root_closed) && !xml_text_is_whitespace(text.as_ref()) =>
            {
                return Err(Error::new("icon", ErrorKind::Malformed));
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return Err(Error::new("icon", ErrorKind::Malformed)),
        }
    }
    if !root_seen || !root_closed || !square || depth != 0 {
        return Err(Error::new("icon", ErrorKind::Malformed));
    }
    Ok(())
}

fn xml_text_is_whitespace(bytes: &[u8]) -> bool {
    bytes.iter().all(u8::is_ascii_whitespace)
}

fn inspect_svg(
    start: &BytesStart<'_>,
    elements: &mut usize,
    attributes: &mut usize,
    root_seen: &mut bool,
    square: &mut bool,
) -> Result<(), Error> {
    *elements = elements.saturating_add(1);
    if *elements > MAX_SVG_ELEMENTS {
        return Err(Error::new("icon", ErrorKind::TooLarge));
    }
    let local_name = start.local_name();
    let name = local_name.as_ref();
    let is_root = !*root_seen;
    if is_root {
        if name != b"svg" {
            return Err(Error::new("icon", ErrorKind::Unsupported));
        }
        *root_seen = true;
    }
    if !matches!(
        name,
        b"svg"
            | b"g"
            | b"path"
            | b"rect"
            | b"circle"
            | b"ellipse"
            | b"line"
            | b"polyline"
            | b"polygon"
            | b"defs"
            | b"linearGradient"
            | b"radialGradient"
            | b"stop"
            | b"title"
            | b"desc"
    ) {
        return Err(Error::new("icon", ErrorKind::UnsafeSvg));
    }
    let mut width = None;
    let mut height = None;
    let mut view_box = None;
    for attribute in start.attributes().with_checks(true) {
        let attribute = attribute.map_err(|_| Error::new("icon", ErrorKind::Malformed))?;
        *attributes = attributes.saturating_add(1);
        if *attributes > MAX_SVG_ATTRIBUTES {
            return Err(Error::new("icon", ErrorKind::TooLarge));
        }
        let local_key = attribute.key.local_name();
        let key = local_key.as_ref();
        if key.starts_with(b"on") || matches!(key, b"href" | b"src" | b"style") {
            return Err(Error::new("icon", ErrorKind::UnsafeSvg));
        }
        let value = std::str::from_utf8(attribute.value.as_ref())
            .map_err(|_| Error::new("icon", ErrorKind::Malformed))?;
        let normalized = value.trim().to_ascii_lowercase();
        let local_fragment = normalized
            .strip_prefix("url(#")
            .and_then(|value| value.strip_suffix(')'))
            .is_some_and(|value| {
                !value.is_empty()
                    && value.chars().all(|character| {
                        character.is_ascii_alphanumeric() || "-_".contains(character)
                    })
            });
        let standard_namespace = key == b"xmlns" && normalized == "http://www.w3.org/2000/svg";
        if (!standard_namespace && normalized.contains("://"))
            || (normalized.contains("url(") && !local_fragment)
            || normalized.contains("@import")
            || normalized.starts_with("data:")
            || normalized.starts_with("javascript:")
        {
            return Err(Error::new("icon", ErrorKind::UnsafeSvg));
        }
        if is_root {
            match key {
                b"width" => {
                    width = Some(
                        svg_length(value)
                            .ok_or_else(|| Error::new("icon", ErrorKind::Malformed))?,
                    );
                }
                b"height" => {
                    height = Some(
                        svg_length(value)
                            .ok_or_else(|| Error::new("icon", ErrorKind::Malformed))?,
                    );
                }
                b"viewBox" => {
                    view_box = Some(
                        square_view_box(value)
                            .ok_or_else(|| Error::new("icon", ErrorKind::Malformed))?,
                    );
                }
                _ => {}
            }
        }
    }
    if is_root {
        *square = match (width, height, view_box) {
            (Some(width), Some(height), _) => approximately_equal(width, height),
            (_, _, Some(square)) => square,
            _ => false,
        };
    }
    Ok(())
}

fn svg_length(value: &str) -> Option<f64> {
    let value = value.trim().strip_suffix("px").unwrap_or(value.trim());
    let value = value.parse::<f64>().ok()?;
    (value.is_finite() && value > 0.0 && value <= 16_384.0).then_some(value)
}

fn square_view_box(value: &str) -> Option<bool> {
    let values = value
        .split(|character: char| character.is_ascii_whitespace() || character == ',')
        .filter(|value| !value.is_empty())
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if values.len() != 4
        || values.iter().any(|value| !value.is_finite())
        || values[2] <= 0.0
        || values[3] <= 0.0
    {
        return None;
    }
    Some(approximately_equal(values[2], values[3]))
}

fn approximately_equal(left: f64, right: f64) -> bool {
    (left - right).abs() <= f64::EPSILON * left.abs().max(right.abs()).max(1.0)
}

pub fn validate_sound_bytes(bytes: &[u8]) -> Result<SoundFormat, Error> {
    if bytes.is_empty() {
        return Err(Error::new("sound", ErrorKind::Empty));
    }
    if bytes.len() > MAX_SOUND_BYTES {
        return Err(Error::new("sound", ErrorKind::TooLarge));
    }
    if bytes.starts_with(b"RIFF") {
        validate_wav(bytes)?;
        return Ok(SoundFormat::WavPcm);
    }
    validate_ogg(bytes)
}

fn validate_wav(bytes: &[u8]) -> Result<(), Error> {
    if bytes.len() < 12 || &bytes[8..12] != b"WAVE" {
        return Err(Error::new("sound", ErrorKind::Unsupported));
    }
    let declared = read_u32_le(bytes, 4)
        .and_then(|length| usize::try_from(length).ok())
        .and_then(|length| length.checked_add(8))
        .ok_or_else(|| Error::new("sound", ErrorKind::Malformed))?;
    if declared != bytes.len() {
        return Err(Error::new("sound", ErrorKind::Malformed));
    }
    let mut cursor = 12_usize;
    let mut byte_rate = None;
    let mut block_align = None;
    let mut data_bytes = None;
    while cursor.checked_add(8).is_some_and(|end| end <= declared) {
        let id = &bytes[cursor..cursor + 4];
        let length = read_u32_le(bytes, cursor + 4)
            .and_then(|length| usize::try_from(length).ok())
            .ok_or_else(|| Error::new("sound", ErrorKind::Malformed))?;
        let start = cursor + 8;
        let end = start
            .checked_add(length)
            .ok_or_else(|| Error::new("sound", ErrorKind::Malformed))?;
        if end > declared {
            return Err(Error::new("sound", ErrorKind::Malformed));
        }
        if id == b"fmt " {
            if length < 16 || byte_rate.is_some() {
                return Err(Error::new("sound", ErrorKind::Malformed));
            }
            let encoding = read_u16_le(bytes, start).unwrap_or(0);
            let channels = read_u16_le(bytes, start + 2).unwrap_or(0);
            let sample_rate = read_u32_le(bytes, start + 4).unwrap_or(0);
            let rate = read_u32_le(bytes, start + 8).unwrap_or(0);
            let align = read_u16_le(bytes, start + 12).unwrap_or(0);
            let bits = read_u16_le(bytes, start + 14).unwrap_or(0);
            if encoding != 1
                || !(1..=2).contains(&channels)
                || !(8_000..=192_000).contains(&sample_rate)
                || !matches!(bits, 8 | 16 | 24 | 32)
                || align == 0
            {
                return Err(Error::new("sound", ErrorKind::Unsupported));
            }
            let expected_align = channels.saturating_mul(bits / 8);
            let expected_rate = sample_rate.saturating_mul(u32::from(expected_align));
            if align != expected_align || rate != expected_rate {
                return Err(Error::new("sound", ErrorKind::Malformed));
            }
            byte_rate = Some(u64::from(rate));
            block_align = Some(u64::from(align));
        } else if id == b"data" {
            if data_bytes.is_some() {
                return Err(Error::new("sound", ErrorKind::Malformed));
            }
            data_bytes = Some(u64::try_from(length).unwrap_or(u64::MAX));
        }
        cursor = end
            .checked_add(length % 2)
            .ok_or_else(|| Error::new("sound", ErrorKind::Malformed))?;
    }
    if cursor != declared {
        return Err(Error::new("sound", ErrorKind::Malformed));
    }
    let rate = byte_rate.ok_or_else(|| Error::new("sound", ErrorKind::Malformed))?;
    let data = data_bytes
        .filter(|bytes| *bytes > 0)
        .ok_or_else(|| Error::new("sound", ErrorKind::Malformed))?;
    if data % block_align.unwrap_or(1) != 0 {
        return Err(Error::new("sound", ErrorKind::Malformed));
    }
    if data > rate.saturating_mul(MAX_SOUND_SECONDS) {
        return Err(Error::new("sound", ErrorKind::TooLarge));
    }
    Ok(())
}

fn validate_ogg(bytes: &[u8]) -> Result<SoundFormat, Error> {
    let mut cursor = 0_usize;
    let mut pages = 0_usize;
    let mut serial = None;
    let mut expected_sequence = None;
    let mut packet_prefix = Vec::new();
    let mut packet_length = 0_usize;
    let mut packets = Vec::new();
    let mut packet_open = false;
    let mut final_granule = None;
    let mut end_seen = false;
    while cursor < bytes.len() {
        if end_seen {
            return Err(Error::new("sound", ErrorKind::Malformed));
        }
        pages = pages.saturating_add(1);
        if pages > MAX_OGG_PAGES || cursor.checked_add(27).is_none_or(|end| end > bytes.len()) {
            return Err(Error::new("sound", ErrorKind::Malformed));
        }
        if &bytes[cursor..cursor + 4] != b"OggS" || bytes[cursor + 4] != 0 {
            return Err(Error::new("sound", ErrorKind::Unsupported));
        }
        let header_type = bytes[cursor + 5];
        let granule = read_u64_le(bytes, cursor + 6)
            .ok_or_else(|| Error::new("sound", ErrorKind::Malformed))?;
        let current_serial = read_u32_le(bytes, cursor + 14)
            .ok_or_else(|| Error::new("sound", ErrorKind::Malformed))?;
        let sequence = read_u32_le(bytes, cursor + 18)
            .ok_or_else(|| Error::new("sound", ErrorKind::Malformed))?;
        if pages == 1 {
            if header_type & 0x02 == 0 || header_type & 0x01 != 0 || sequence != 0 {
                return Err(Error::new("sound", ErrorKind::Malformed));
            }
        } else if header_type & 0x02 != 0 || (header_type & 0x01 != 0) != packet_open {
            return Err(Error::new("sound", ErrorKind::Malformed));
        }
        if header_type & !0x07 != 0 {
            return Err(Error::new("sound", ErrorKind::Malformed));
        }
        if serial
            .replace(current_serial)
            .is_some_and(|value| value != current_serial)
            || expected_sequence.is_some_and(|value| value != sequence)
        {
            return Err(Error::new("sound", ErrorKind::Unsupported));
        }
        expected_sequence = sequence.checked_add(1);
        let segment_count = usize::from(bytes[cursor + 26]);
        let table_start = cursor + 27;
        let table_end = table_start + segment_count;
        if table_end > bytes.len() {
            return Err(Error::new("sound", ErrorKind::Malformed));
        }
        let payload_len = bytes[table_start..table_end]
            .iter()
            .try_fold(0_usize, |total, length| {
                total.checked_add(usize::from(*length))
            })
            .ok_or_else(|| Error::new("sound", ErrorKind::Malformed))?;
        let payload_end = table_end
            .checked_add(payload_len)
            .ok_or_else(|| Error::new("sound", ErrorKind::Malformed))?;
        if payload_end > bytes.len() {
            return Err(Error::new("sound", ErrorKind::Malformed));
        }
        let expected_checksum = read_u32_le(bytes, cursor + 22)
            .ok_or_else(|| Error::new("sound", ErrorKind::Malformed))?;
        if ogg_checksum(&bytes[cursor..payload_end]) != expected_checksum {
            return Err(Error::new("sound", ErrorKind::Malformed));
        }
        let mut offset = table_end;
        for length in &bytes[table_start..table_end] {
            let length = usize::from(*length);
            let end = offset + length;
            packet_length = packet_length
                .checked_add(length)
                .ok_or_else(|| Error::new("sound", ErrorKind::TooLarge))?;
            if packets.len() < 4 && packet_prefix.len() < MAX_OGG_PACKET_PREFIX {
                let retained = (MAX_OGG_PACKET_PREFIX - packet_prefix.len()).min(length);
                packet_prefix.extend_from_slice(&bytes[offset..offset + retained]);
            }
            offset = end;
            packet_open = length == 255;
            if !packet_open {
                if packets.len() < 4 {
                    packets.push((std::mem::take(&mut packet_prefix), packet_length));
                }
                packet_length = 0;
            }
        }
        if granule != u64::MAX {
            final_granule = Some(granule);
        }
        end_seen |= header_type & 0x04 != 0;
        cursor = payload_end;
    }
    if packet_open || !end_seen || cursor != bytes.len() {
        return Err(Error::new("sound", ErrorKind::Malformed));
    }
    let Some((identification, identification_length)) = packets.first() else {
        return Err(Error::new("sound", ErrorKind::Malformed));
    };
    let (format, sample_rate, preskip) = if identification.starts_with(b"OpusHead") {
        if *identification_length < 19
            || identification.len() < 19
            || identification[8] != 1
            || !(1..=2).contains(&identification[9])
            || identification[18] != 0
            || packets.len() < 3
            || !packets[1].0.starts_with(b"OpusTags")
            || packets[2].1 == 0
        {
            return Err(Error::new("sound", ErrorKind::Unsupported));
        }
        (
            SoundFormat::OggOpus,
            48_000_u64,
            u64::from(read_u16_le(identification, 10).unwrap_or(0)),
        )
    } else if identification.starts_with(b"\x01vorbis") {
        if *identification_length < 30
            || identification.len() < 30
            || identification[7..11] != [0; 4]
            || !(1..=2).contains(&identification[11])
            || identification[29] & 1 == 0
            || packets.len() < 4
            || !packets[1].0.starts_with(b"\x03vorbis")
            || !packets[2].0.starts_with(b"\x05vorbis")
            || packets[3].1 == 0
        {
            return Err(Error::new("sound", ErrorKind::Unsupported));
        }
        let sample_rate = read_u32_le(identification, 12).unwrap_or(0);
        if !(8_000..=192_000).contains(&sample_rate) {
            return Err(Error::new("sound", ErrorKind::Unsupported));
        }
        (SoundFormat::OggVorbis, u64::from(sample_rate), 0)
    } else {
        return Err(Error::new("sound", ErrorKind::Unsupported));
    };
    let granule = final_granule.ok_or_else(|| Error::new("sound", ErrorKind::Malformed))?;
    if format == SoundFormat::OggOpus && granule < preskip {
        return Err(Error::new("sound", ErrorKind::Malformed));
    }
    if granule.saturating_sub(preskip) > sample_rate.saturating_mul(MAX_SOUND_SECONDS) {
        return Err(Error::new("sound", ErrorKind::TooLarge));
    }
    Ok(format)
}

fn ogg_checksum(page: &[u8]) -> u32 {
    let mut checksum = 0_u32;
    for (index, byte) in page.iter().copied().enumerate() {
        let byte = if (22..26).contains(&index) { 0 } else { byte };
        checksum ^= u32::from(byte) << 24;
        for _ in 0..8 {
            checksum = if checksum & 0x8000_0000 == 0 {
                checksum << 1
            } else {
                (checksum << 1) ^ 0x04c1_1db7
            };
        }
    }
    checksum
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn read_u64_le(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        bytes.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

#[cfg(unix)]
fn read_descriptor(
    payload: OwnedValue,
    field: &'static str,
    limit: usize,
) -> Result<Vec<u8>, Error> {
    let descriptor = Value::from(payload)
        .downcast::<zbus::zvariant::Fd<'static>>()
        .map_err(|_| Error::new(field, ErrorKind::WrongType))?;
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd as _;

        // SAFETY: zvariant owns a live descriptor for this scope; F_GET_SEALS
        // only queries immutable kernel metadata and writes no user memory.
        let seals = unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_GET_SEALS) };
        let required = libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_WRITE;
        if seals < 0 || seals & required != required {
            return Err(Error::new(field, ErrorKind::Unsealed));
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = descriptor;
        let _ = limit;
        Err(Error::new(field, ErrorKind::Unsupported))
    }
    #[cfg(target_os = "linux")]
    {
        use std::fs::File;
        use std::os::fd::OwnedFd;
        use std::os::unix::fs::FileExt as _;

        let owned =
            OwnedFd::try_from(descriptor).map_err(|_| Error::new(field, ErrorKind::WrongType))?;
        let file = File::from(owned);
        let metadata = file
            .metadata()
            .map_err(|_| Error::new(field, ErrorKind::Malformed))?;
        if !metadata.is_file() {
            return Err(Error::new(field, ErrorKind::Unsupported));
        }
        let byte_len =
            usize::try_from(metadata.len()).map_err(|_| Error::new(field, ErrorKind::TooLarge))?;
        if byte_len == 0 {
            return Err(Error::new(field, ErrorKind::Empty));
        }
        if byte_len > limit {
            return Err(Error::new(field, ErrorKind::TooLarge));
        }
        let mut bytes = vec![0; byte_len];
        file.read_exact_at(&mut bytes, 0)
            .map_err(|_| Error::new(field, ErrorKind::Malformed))?;
        Ok(bytes)
    }
}

#[cfg(not(unix))]
fn read_descriptor(
    _payload: OwnedValue,
    field: &'static str,
    _limit: usize,
) -> Result<Vec<u8>, Error> {
    Err(Error::new(field, ErrorKind::Unsupported))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgba, RgbaImage};
    use zbus::zvariant::Str;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            width,
            height,
            Rgba([40, 110, 220, 255]),
        ));
        let mut output = Cursor::new(Vec::new());
        image.write_to(&mut output, ImageFormat::Png).unwrap();
        output.into_inner()
    }

    fn pcm_wav(seconds: u32) -> Vec<u8> {
        let sample_rate = 8_000_u32;
        let data_len = sample_rate.saturating_mul(seconds);
        let mut bytes = Vec::with_capacity(44 + usize::try_from(data_len).unwrap());
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&8_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        bytes.resize(44 + usize::try_from(data_len).unwrap(), 128);
        bytes
    }

    fn ogg_page(packets: &[&[u8]], granule: u64) -> Vec<u8> {
        assert!(packets.iter().all(|packet| packet.len() < 255));
        let payload_len = packets.iter().map(|packet| packet.len()).sum::<usize>();
        let mut bytes = vec![0_u8; 27 + packets.len() + payload_len];
        bytes[..4].copy_from_slice(b"OggS");
        bytes[5] = 0x06;
        bytes[6..14].copy_from_slice(&granule.to_le_bytes());
        bytes[14..18].copy_from_slice(&1_u32.to_le_bytes());
        bytes[26] = u8::try_from(packets.len()).unwrap();
        let mut cursor = 27 + packets.len();
        for (index, packet) in packets.iter().enumerate() {
            bytes[27 + index] = u8::try_from(packet.len()).unwrap();
            bytes[cursor..cursor + packet.len()].copy_from_slice(packet);
            cursor += packet.len();
        }
        let checksum = ogg_checksum(&bytes);
        bytes[22..26].copy_from_slice(&checksum.to_le_bytes());
        bytes
    }

    fn opus() -> Vec<u8> {
        let mut head = vec![0_u8; 19];
        head[..8].copy_from_slice(b"OpusHead");
        head[8] = 1;
        head[9] = 2;
        head[10..12].copy_from_slice(&312_u16.to_le_bytes());
        head[12..16].copy_from_slice(&48_000_u32.to_le_bytes());
        let tags = b"OpusTags\0\0\0\0\0\0\0\0";
        ogg_page(&[&head, tags, b"\xf8\xff\xfe"], 48_312)
    }

    fn vorbis() -> Vec<u8> {
        let mut identification = vec![0_u8; 30];
        identification[0] = 1;
        identification[1..7].copy_from_slice(b"vorbis");
        identification[11] = 2;
        identification[12..16].copy_from_slice(&44_100_u32.to_le_bytes());
        identification[29] = 1;
        ogg_page(
            &[&identification, b"\x03vorbis", b"\x05vorbis", b"\x00"],
            44_100,
        )
    }

    #[test]
    fn raster_icons_are_fully_decoded_and_must_be_square() {
        assert_eq!(validate_icon_bytes(&png(32, 32)), Ok(IconFormat::Png));
        assert_eq!(
            validate_icon_bytes(&png(32, 16)).unwrap_err().kind,
            ErrorKind::Malformed
        );
        assert_eq!(
            validate_icon_bytes(&png(MAX_ICON_EDGE + 1, MAX_ICON_EDGE + 1))
                .unwrap_err()
                .kind,
            ErrorKind::TooLarge
        );
    }

    #[test]
    fn svg_icons_use_a_square_non_executable_profile() {
        let safe = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><path d="M0 0h32v32z"/></svg>"#;
        assert_eq!(validate_icon_bytes(safe), Ok(IconFormat::Svg));

        let script = br#"<svg viewBox="0 0 32 32"><script>alert(1)</script></svg>"#;
        assert_eq!(
            validate_icon_bytes(script).unwrap_err().kind,
            ErrorKind::UnsafeSvg
        );
        let remote = br#"<svg viewBox="0 0 32 32"><path fill="url(https://example.test/a)" d="M0 0"/></svg>"#;
        assert_eq!(
            validate_icon_bytes(remote).unwrap_err().kind,
            ErrorKind::UnsafeSvg
        );
        let nonsquare = br#"<svg viewBox="0 0 32 16"><path d="M0 0"/></svg>"#;
        assert_eq!(
            validate_icon_bytes(nonsquare).unwrap_err().kind,
            ErrorKind::Malformed
        );
    }

    #[test]
    fn wav_duration_and_pcm_layout_are_bounded() {
        assert_eq!(validate_sound_bytes(&pcm_wav(1)), Ok(SoundFormat::WavPcm));
        let mut invalid_rate = pcm_wav(1);
        invalid_rate[28..32].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            validate_sound_bytes(&invalid_rate).unwrap_err().kind,
            ErrorKind::Malformed
        );
        assert_eq!(
            validate_sound_bytes(&pcm_wav(u32::try_from(MAX_SOUND_SECONDS + 1).unwrap()))
                .unwrap_err()
                .kind,
            ErrorKind::TooLarge
        );
    }

    #[test]
    fn ogg_sound_headers_sequences_checksums_and_duration_are_validated() {
        assert_eq!(validate_sound_bytes(&opus()), Ok(SoundFormat::OggOpus));
        assert_eq!(validate_sound_bytes(&vorbis()), Ok(SoundFormat::OggVorbis));
        let mut corrupt = opus();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 1;
        assert_eq!(
            validate_sound_bytes(&corrupt).unwrap_err().kind,
            ErrorKind::Malformed
        );
    }

    #[test]
    fn themed_icons_are_bounded_and_diagnostics_are_redacted() {
        let icon = take_icon(OwnedValue::from(Str::from("private-icon-name-8472"))).unwrap();
        assert!(matches!(icon, Some(Icon::Themed(_))));
        assert!(!format!("{icon:?}").contains("private-icon-name-8472"));
        assert_eq!(
            validate_icon_names(vec!["../secret".into()])
                .unwrap_err()
                .kind,
            ErrorKind::Malformed
        );
    }

    #[test]
    fn delivery_policy_drops_media_that_cannot_be_presented() {
        let media = NotificationMedia {
            icon: Some(Icon::Themed(vec!["mail-unread".into()])),
            sound: Some(CustomSound {
                format: SoundFormat::WavPcm,
                bytes: pcm_wav(1).into(),
            }),
        };
        let mut banner_only = media.clone();
        banner_only.retain_for(rmac_notifications::Delivery {
            banner: true,
            history: true,
            sound: false,
        });
        assert!(banner_only.icon.is_some());
        assert!(banner_only.sound.is_none());

        let mut media = media;
        media.retain_for(rmac_notifications::Delivery {
            banner: false,
            history: true,
            sound: false,
        });
        assert_eq!(media, NotificationMedia::default());
    }

    #[cfg(target_os = "linux")]
    fn descriptor_media(kind: &str, bytes: &[u8], sealed: bool) -> OwnedValue {
        use std::ffi::CString;
        use std::fs::File;
        use std::io::Write as _;
        use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};

        let name = CString::new("rmac-notification-media-test").unwrap();
        // SAFETY: the name is a live NUL-terminated C string and the returned
        // descriptor is checked before ownership is assumed.
        let raw = unsafe {
            libc::memfd_create(name.as_ptr(), libc::MFD_ALLOW_SEALING | libc::MFD_CLOEXEC)
        };
        assert!(raw >= 0);
        // SAFETY: memfd_create returned a fresh descriptor that this File owns.
        let mut file = File::from(unsafe { OwnedFd::from_raw_fd(raw) });
        file.write_all(bytes).unwrap();
        if sealed {
            let seals = libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_WRITE;
            // SAFETY: fcntl only applies immutable seals to this live memfd.
            assert_eq!(
                unsafe { libc::fcntl(file.as_raw_fd(), libc::F_ADD_SEALS, seals) },
                0
            );
        }
        let descriptor = zbus::zvariant::Fd::from(OwnedFd::from(file));
        let payload = OwnedValue::try_from(Value::new(descriptor)).unwrap();
        OwnedValue::try_from(Value::new((kind.to_owned(), payload))).unwrap()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn sealed_descriptors_freeze_icons_and_sounds_but_unsealed_inputs_fail() {
        let icon = take_icon(descriptor_media("file-descriptor", &png(32, 32), true)).unwrap();
        assert!(matches!(
            icon,
            Some(Icon::File {
                format: IconFormat::Png,
                ..
            })
        ));
        let sound = take_sound(descriptor_media("file-descriptor", &pcm_wav(1), true)).unwrap();
        assert!(matches!(
            sound,
            SoundValue::Custom(CustomSound {
                format: SoundFormat::WavPcm,
                ..
            })
        ));
        assert_eq!(
            take_icon(descriptor_media("file-descriptor", &png(32, 32), false))
                .unwrap_err()
                .kind,
            ErrorKind::Unsealed
        );
    }
}
