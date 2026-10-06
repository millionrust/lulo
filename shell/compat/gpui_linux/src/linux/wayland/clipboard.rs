use std::{
    fs::File,
    io::{ErrorKind, Write},
    os::fd::{AsRawFd, BorrowedFd, OwnedFd},
};

use calloop::{LoopHandle, PostAction};
use filedescriptor::Pipe;
use strum::IntoEnumIterator;
use wayland_client::{Connection, protocol::wl_data_offer::WlDataOffer};
use wayland_protocols::wp::primary_selection::zv1::client::zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1;

use crate::linux::{
    WaylandClientStatePtr,
    platform::{PIPE_READ_TIMEOUT, read_fd_with_timeout},
};
use gpui::{ClipboardEntry, ClipboardItem, Image, ImageFormat, hash};

/// Text mime types that we'll offer to other programs.
pub(crate) const TEXT_MIME_TYPES: [&str; 3] =
    ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];
pub(crate) const FILE_LIST_MIME_TYPE: &str = "text/uri-list";

/// Text mime types that we'll accept from other programs.
pub(crate) const ALLOWED_TEXT_MIME_TYPES: [&str; 2] = ["text/plain;charset=utf-8", "UTF8_STRING"];

/// rmac: rich flavours ride in a string entry's metadata, framed as
/// `crates/rmac-editor/src/rich/clipboard.rs` writes them. Each is offered
/// under its own MIME type next to the plain text.
const RICH_FORMATS_HEADER: &str = "x-lulo-clipboard-formats/1\n";
/// The rich flavours offered and accepted, and the MIME names other apps
/// also use for them.
const RICH_MIME_ALIASES: [(&str, &[&str]); 2] = [
    (
        "text/rtf",
        &["text/rtf", "application/rtf", "text/richtext"],
    ),
    ("text/html", &["text/html"]),
];
/// The largest rich flavour read from another app.
const MAX_RICH_BYTES: usize = 16 * 1024 * 1024;

/// The flavours framed in clipboard metadata (see [`RICH_FORMATS_HEADER`]).
pub(crate) fn rich_formats(metadata: &str) -> Vec<(&str, &str)> {
    let Some(mut rest) = metadata.strip_prefix(RICH_FORMATS_HEADER) else {
        return Vec::new();
    };
    let mut formats = Vec::new();
    while !rest.is_empty() {
        let Some((mime, after)) = rest.split_once('\n') else {
            break;
        };
        let Some((length, after)) = after.split_once('\n') else {
            break;
        };
        let Ok(length) = length.parse::<usize>() else {
            break;
        };
        if length > after.len() || !after.is_char_boundary(length) {
            break;
        }
        formats.push((mime, &after[..length]));
        rest = &after[length..];
    }
    formats
}

fn frame_rich_formats(formats: &[(&str, &str)]) -> String {
    let mut out = String::from(RICH_FORMATS_HEADER);
    for (mime, payload) in formats {
        out.push_str(mime);
        out.push('\n');
        out.push_str(&payload.len().to_string());
        out.push('\n');
        out.push_str(payload);
    }
    out
}

/// Every MIME type `item`'s rich flavours are offered under.
pub(crate) fn rich_mime_types(item: &ClipboardItem) -> Vec<&'static str> {
    let Some(metadata) = item.metadata() else {
        return Vec::new();
    };
    let mut types = Vec::new();
    for (mime, _) in rich_formats(metadata) {
        if let Some((_, aliases)) = RICH_MIME_ALIASES.iter().find(|(name, _)| *name == mime) {
            types.extend_from_slice(aliases);
        }
    }
    types
}

/// The bytes `item` sends for `mime_type`: a rich flavour when asked for
/// one it has, the plain text otherwise.
fn payload_for(item: &ClipboardItem, mime_type: &str) -> Option<Vec<u8>> {
    let rich = item.metadata().and_then(|metadata| {
        let (canonical, _) = RICH_MIME_ALIASES
            .iter()
            .find(|(_, aliases)| aliases.contains(&mime_type))?;
        rich_formats(metadata)
            .into_iter()
            .find(|(mime, _)| mime == canonical)
            .map(|(_, payload)| payload.as_bytes().to_owned())
    });
    rich.or_else(|| item.text().map(|text| text.into_bytes()))
}

pub(crate) struct Clipboard {
    connection: Connection,
    loop_handle: LoopHandle<'static, WaylandClientStatePtr>,
    self_mime: String,

    // Internal clipboard
    contents: Option<ClipboardItem>,
    primary_contents: Option<ClipboardItem>,

    // External clipboard
    cached_read: Option<ClipboardItem>,
    current_offer: Option<DataOffer<WlDataOffer>>,
    cached_primary_read: Option<ClipboardItem>,
    current_primary_offer: Option<DataOffer<ZwpPrimarySelectionOfferV1>>,
}

pub(crate) trait ReceiveData {
    fn receive_data(&self, mime_type: String, fd: BorrowedFd<'_>);
}

impl ReceiveData for WlDataOffer {
    fn receive_data(&self, mime_type: String, fd: BorrowedFd<'_>) {
        self.receive(mime_type, fd);
    }
}

impl ReceiveData for ZwpPrimarySelectionOfferV1 {
    fn receive_data(&self, mime_type: String, fd: BorrowedFd<'_>) {
        self.receive(mime_type, fd);
    }
}

#[derive(Clone, Debug)]
/// Wrapper for `WlDataOffer` and `ZwpPrimarySelectionOfferV1`, used to help track mime types.
pub(crate) struct DataOffer<T: ReceiveData> {
    pub inner: T,
    mime_types: Vec<String>,
}

impl<T: ReceiveData> DataOffer<T> {
    pub fn new(offer: T) -> Self {
        Self {
            inner: offer,
            mime_types: Vec::new(),
        }
    }

    pub fn add_mime_type(&mut self, mime_type: String) {
        self.mime_types.push(mime_type)
    }

    fn has_mime_type(&self, mime_type: &str) -> bool {
        self.mime_types.iter().any(|t| t == mime_type)
    }

    fn read_bytes(&self, connection: &Connection, mime_type: &str) -> Option<Vec<u8>> {
        let pipe = Pipe::new().unwrap();
        self.inner.receive_data(mime_type.to_string(), unsafe {
            BorrowedFd::borrow_raw(pipe.write.as_raw_fd())
        });
        let fd = pipe.read;
        drop(pipe.write);

        connection.flush().unwrap();

        match read_fd_with_timeout(fd, PIPE_READ_TIMEOUT) {
            Ok(bytes) => Some(bytes),
            Err(err) => {
                log::error!("error reading clipboard pipe: {err:?}");
                None
            }
        }
    }

    fn read_text(&self, connection: &Connection) -> Option<ClipboardItem> {
        let mime_type = self.mime_types.iter().find(|&mime_type| {
            ALLOWED_TEXT_MIME_TYPES
                .iter()
                .any(|&allowed| allowed == mime_type)
        })?;
        let bytes = self.read_bytes(connection, mime_type)?;
        let text_content = match String::from_utf8(bytes) {
            Ok(content) => content,
            Err(e) => {
                log::error!("Failed to convert clipboard content to UTF-8: {}", e);
                return None;
            }
        };

        // Normalize the text to unix line endings, otherwise
        // copying from eg: firefox inserts a lot of blank
        // lines, and that is super annoying.
        let result = text_content.replace("\r\n", "\n");
        // rmac: another app's RTF flavour travels with the text, framed in
        // the metadata, so a rich editor can paste it with its formatting.
        let rtf = RICH_MIME_ALIASES[0]
            .1
            .iter()
            .find(|alias| self.has_mime_type(alias))
            .and_then(|alias| self.read_bytes(connection, alias))
            .filter(|bytes| bytes.len() <= MAX_RICH_BYTES)
            .and_then(|bytes| String::from_utf8(bytes).ok());
        Some(match rtf {
            Some(rtf) => ClipboardItem::new_string_with_metadata(
                result,
                frame_rich_formats(&[("text/rtf", rtf.as_str())]),
            ),
            None => ClipboardItem::new_string(result),
        })
    }

    fn read_image(&self, connection: &Connection) -> Option<ClipboardItem> {
        for format in ImageFormat::iter() {
            let mime_type = format.mime_type();
            if !self.has_mime_type(mime_type) {
                continue;
            }

            if let Some(bytes) = self.read_bytes(connection, mime_type) {
                let id = hash(&bytes);
                return Some(ClipboardItem {
                    entries: vec![ClipboardEntry::Image(Image { format, bytes, id })],
                });
            }
        }
        None
    }
}

impl Clipboard {
    pub fn new(
        connection: Connection,
        loop_handle: LoopHandle<'static, WaylandClientStatePtr>,
    ) -> Self {
        Self {
            connection,
            loop_handle,
            self_mime: format!("pid/{}", std::process::id()),

            contents: None,
            primary_contents: None,

            cached_read: None,
            current_offer: None,
            cached_primary_read: None,
            current_primary_offer: None,
        }
    }

    pub fn set(&mut self, item: ClipboardItem) {
        self.contents = Some(item);
    }

    pub fn set_primary(&mut self, item: ClipboardItem) {
        self.primary_contents = Some(item);
    }

    pub fn set_offer(&mut self, data_offer: Option<DataOffer<WlDataOffer>>) {
        self.cached_read = None;
        self.current_offer = data_offer;
    }

    pub fn set_primary_offer(&mut self, data_offer: Option<DataOffer<ZwpPrimarySelectionOfferV1>>) {
        self.cached_primary_read = None;
        self.current_primary_offer = data_offer;
    }

    pub fn self_mime(&self) -> String {
        self.self_mime.clone()
    }

    pub fn send(&self, mime_type: String, fd: OwnedFd) {
        if let Some(bytes) = self
            .contents
            .as_ref()
            .and_then(|contents| payload_for(contents, &mime_type))
        {
            self.send_internal(fd, bytes);
        }
    }

    pub fn send_primary(&self, _mime_type: String, fd: OwnedFd) {
        if let Some(text) = self
            .primary_contents
            .as_ref()
            .and_then(|contents| contents.text())
        {
            self.send_internal(fd, text.as_bytes().to_owned());
        }
    }

    pub fn read(&mut self) -> Option<ClipboardItem> {
        let offer = self.current_offer.as_ref()?;
        if let Some(cached) = self.cached_read.clone() {
            return Some(cached);
        }

        if offer.has_mime_type(&self.self_mime) {
            return self.contents.clone();
        }

        let item = offer
            .read_text(&self.connection)
            .or_else(|| offer.read_image(&self.connection))?;

        self.cached_read = Some(item.clone());
        Some(item)
    }

    pub fn read_primary(&mut self) -> Option<ClipboardItem> {
        let offer = self.current_primary_offer.as_ref()?;
        if let Some(cached) = self.cached_primary_read.clone() {
            return Some(cached);
        }

        if offer.has_mime_type(&self.self_mime) {
            return self.primary_contents.clone();
        }

        let item = offer
            .read_text(&self.connection)
            .or_else(|| offer.read_image(&self.connection))?;

        self.cached_primary_read = Some(item.clone());
        Some(item)
    }

    fn send_internal(&self, fd: OwnedFd, bytes: Vec<u8>) {
        let mut written = 0;
        self.loop_handle
            .insert_source(
                calloop::generic::Generic::new(
                    File::from(fd),
                    calloop::Interest::WRITE,
                    calloop::Mode::Level,
                ),
                move |_, file, _| {
                    let file = unsafe { file.get_mut() };
                    loop {
                        match file.write(&bytes[written..]) {
                            Ok(n) if written + n == bytes.len() => {
                                written += n;
                                break Ok(PostAction::Remove);
                            }
                            Ok(n) => written += n,
                            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                                break Ok(PostAction::Continue);
                            }
                            Err(_) => break Ok(PostAction::Remove),
                        }
                    }
                },
            )
            .unwrap();
    }
}
