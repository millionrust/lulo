use std::collections::HashMap;
use std::fmt;
use std::io::{Seek as _, Write as _};
use std::os::fd::AsFd as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures_util::{future::Either, StreamExt as _};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
use rmac_print::{
    OutputFormat, PageDescription, PageOrientation, PortalPrintTransaction, PrintIdentity,
};
use zbus::zvariant::{Fd, OwnedObjectPath, OwnedValue, Value};

const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const PRINT_INTERFACE: &str = "org.freedesktop.portal.Print";
const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";
const REQUIRED_PORTAL_VERSION: u32 = 3;
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct PrintDocument {
    pub window: RawWindowHandle,
    pub display: RawDisplayHandle,
    pub window_generation: u64,
    pub document_generation: u64,
    pub current_document_generation: Arc<AtomicU64>,
    pub title: String,
    pub text: String,
}

impl fmt::Debug for PrintDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrintDocument")
            .field("window", &"<private>")
            .field("display", &"<private>")
            .field("window_generation", &self.window_generation)
            .field("document_generation", &self.document_generation)
            .field("title", &"<private>")
            .field("text", &"<private>")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Printed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ErrorKind {
    ParentWindow,
    PortalUnavailable,
    PortalTooOld,
    InvalidResponse,
    StaleDocument,
    Render,
    Descriptor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
}

impl Error {
    fn new(kind: ErrorKind) -> Self {
        Self { kind }
    }

    pub fn is_stale_document(&self) -> bool {
        self.kind == ErrorKind::StaleDocument
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            ErrorKind::ParentWindow => {
                "printing requires the current exported Wayland application window"
            }
            ErrorKind::PortalUnavailable => "the desktop print service is temporarily unavailable",
            ErrorKind::PortalTooOld => {
                "the desktop print service cannot restrict print-to-file output to PDF"
            }
            ErrorKind::InvalidResponse => {
                "the desktop print service returned unsupported print settings"
            }
            ErrorKind::StaleDocument => {
                "the document changed while the print dialog was open; nothing was printed"
            }
            ErrorKind::Render => "Text Editor could not render this document as a safe PDF",
            ErrorKind::Descriptor => "Text Editor could not create the private printable document",
        })
    }
}

impl std::error::Error for Error {}

enum PortalResponse {
    Accepted(HashMap<String, OwnedValue>),
    Cancelled,
}

/// Run the complete PreparePrint → PDF render → Print transaction.
///
/// The caller must keep the GPUI window alive while this future is pending.
/// Text Editor enforces that by treating printing as a close-blocking modal
/// operation. The exported identifier remains alive through both portal calls.
pub async fn print_document(request: PrintDocument) -> Result<Outcome, Error> {
    let parent = ashpd::WindowIdentifier::from_raw_handle(&request.window, Some(&request.display))
        .await
        .ok_or_else(|| Error::new(ErrorKind::ParentWindow))?;
    let parent_string = parent.to_string();
    let identity = PrintIdentity::new(
        parent_string.clone(),
        request.window_generation,
        request.document_generation,
    )
    .map_err(|_| Error::new(ErrorKind::ParentWindow))?;
    require_current(&request, &identity, &parent_string)?;
    let mut transaction = PortalPrintTransaction::new(identity.clone(), request.title.clone())
        .map_err(|_| Error::new(ErrorKind::InvalidResponse))?;

    let connection = zbus::Connection::session()
        .await
        .map_err(|_| Error::new(ErrorKind::PortalUnavailable))?;
    let portal = zbus::Proxy::new(&connection, DESTINATION, PORTAL_PATH, PRINT_INTERFACE)
        .await
        .map_err(|_| Error::new(ErrorKind::PortalUnavailable))?;
    let version = portal
        .get_property::<u32>("version")
        .await
        .map_err(|_| Error::new(ErrorKind::PortalUnavailable))?;
    if version < REQUIRED_PORTAL_VERSION {
        return Err(Error::new(ErrorKind::PortalTooOld));
    }

    let prepared = prepare_print(
        &connection,
        &portal,
        &parent_string,
        transaction.title(),
        transaction.supported_output_file_formats(),
    )
    .await?;
    let values = match prepared {
        PortalResponse::Accepted(values) => values,
        PortalResponse::Cancelled => {
            transaction
                .cancel(&identity)
                .map_err(|_| Error::new(ErrorKind::InvalidResponse))?;
            return Ok(Outcome::Cancelled);
        }
    };
    let current = current_identity(&request, &parent_string)?;
    let (token, page, format) = decode_prepared(values)?;
    let layout = transaction
        .prepared(&current, token, page, format.as_deref())
        .map_err(|error| {
            if matches!(error, rmac_print::PortalPrintError::StaleIdentity) {
                Error::new(ErrorKind::StaleDocument)
            } else {
                Error::new(ErrorKind::InvalidResponse)
            }
        })?;

    let text = request.text.clone();
    let pdf = blocking::unblock(move || rmac_print::render_pdf(&text, layout))
        .await
        .map_err(|_| Error::new(ErrorKind::Render))?;
    let current = current_identity(&request, &parent_string)?;
    transaction
        .rendered(&current, layout, OutputFormat::Pdf)
        .map_err(|error| {
            if matches!(error, rmac_print::PortalPrintError::StaleIdentity) {
                Error::new(ErrorKind::StaleDocument)
            } else {
                Error::new(ErrorKind::InvalidResponse)
            }
        })?;
    let descriptor = blocking::unblock(move || printable_descriptor(&pdf))
        .await
        .map_err(|_| Error::new(ErrorKind::Descriptor))?;
    let current = current_identity(&request, &parent_string)?;
    let submission = transaction.submission(&current).map_err(|error| {
        if matches!(error, rmac_print::PortalPrintError::StaleIdentity) {
            Error::new(ErrorKind::StaleDocument)
        } else {
            Error::new(ErrorKind::InvalidResponse)
        }
    })?;

    match submit_print(&connection, &portal, &submission, &descriptor).await? {
        PortalResponse::Accepted(_) => {
            transaction
                .finish(&identity)
                .map_err(|_| Error::new(ErrorKind::InvalidResponse))?;
            Ok(Outcome::Printed)
        }
        PortalResponse::Cancelled => {
            transaction
                .cancel(&identity)
                .map_err(|_| Error::new(ErrorKind::InvalidResponse))?;
            Ok(Outcome::Cancelled)
        }
    }
}

fn require_current(
    request: &PrintDocument,
    expected: &PrintIdentity,
    parent: &str,
) -> Result<(), Error> {
    let current = current_identity(request, parent)?;
    (&current == expected)
        .then_some(())
        .ok_or_else(|| Error::new(ErrorKind::StaleDocument))
}

fn current_identity(request: &PrintDocument, parent: &str) -> Result<PrintIdentity, Error> {
    PrintIdentity::new(
        parent,
        request.window_generation,
        request.current_document_generation.load(Ordering::Acquire),
    )
    .map_err(|_| Error::new(ErrorKind::ParentWindow))
}

async fn prepare_print(
    connection: &zbus::Connection,
    portal: &zbus::Proxy<'_>,
    parent: &str,
    title: &str,
    formats: &[&str],
) -> Result<PortalResponse, Error> {
    let token = next_handle_token("prepare");
    let options = prepare_options(&token, formats)?;
    let settings = HashMap::<String, OwnedValue>::new();
    let page_setup = HashMap::<String, OwnedValue>::new();
    request(
        connection,
        portal,
        "PreparePrint",
        &token,
        &(parent, title, settings, page_setup, options),
    )
    .await
}

async fn submit_print(
    connection: &zbus::Connection,
    portal: &zbus::Proxy<'_>,
    submission: &rmac_print::PrintSubmission,
    descriptor: &std::fs::File,
) -> Result<PortalResponse, Error> {
    let token = next_handle_token("submit");
    let options = submission_options(&token, submission)?;
    request(
        connection,
        portal,
        "Print",
        &token,
        &(
            submission.parent_window.as_str(),
            submission.title.as_str(),
            Fd::from(descriptor.as_fd()),
            options,
        ),
    )
    .await
}

async fn request<B>(
    connection: &zbus::Connection,
    portal: &zbus::Proxy<'_>,
    method: &str,
    token: &str,
    body: &B,
) -> Result<PortalResponse, Error>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let path = request_path(connection, token)?;
    let request = zbus::Proxy::new(connection, DESTINATION, path.clone(), REQUEST_INTERFACE)
        .await
        .map_err(|_| Error::new(ErrorKind::PortalUnavailable))?;
    let mut responses = request
        .receive_signal("Response")
        .await
        .map_err(|_| Error::new(ErrorKind::PortalUnavailable))?;
    let mut owner_changes = portal
        .receive_owner_changed()
        .await
        .map_err(|_| Error::new(ErrorKind::PortalUnavailable))?;
    let returned = portal
        .call::<_, _, OwnedObjectPath>(method, body)
        .await
        .map_err(|_| Error::new(ErrorKind::PortalUnavailable))?;
    if returned != path {
        return Err(Error::new(ErrorKind::InvalidResponse));
    }
    let response = match futures_util::future::select(responses.next(), owner_changes.next()).await
    {
        Either::Left((response, _)) => response,
        Either::Right(_) => return Err(Error::new(ErrorKind::PortalUnavailable)),
    }
    .ok_or_else(|| Error::new(ErrorKind::PortalUnavailable))?
    .body()
    .deserialize::<(u32, HashMap<String, OwnedValue>)>()
    .map_err(|_| Error::new(ErrorKind::InvalidResponse))?;
    match response.0 {
        0 => Ok(PortalResponse::Accepted(response.1)),
        1 => Ok(PortalResponse::Cancelled),
        _ => Err(Error::new(ErrorKind::PortalUnavailable)),
    }
}

fn request_path(connection: &zbus::Connection, token: &str) -> Result<OwnedObjectPath, Error> {
    let sender = connection
        .unique_name()
        .ok_or_else(|| Error::new(ErrorKind::PortalUnavailable))?
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    OwnedObjectPath::try_from(format!(
        "/org/freedesktop/portal/desktop/request/{sender}/{token}"
    ))
    .map_err(|_| Error::new(ErrorKind::InvalidResponse))
}

fn next_handle_token(kind: &str) -> String {
    let value = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    format!("rmac_{kind}_{}_{value}", std::process::id())
}

fn base_options(token: &str) -> Result<HashMap<String, OwnedValue>, Error> {
    Ok(HashMap::from([
        (
            "handle_token".into(),
            owned_value(token.to_string()).map_err(|_| Error::new(ErrorKind::InvalidResponse))?,
        ),
        (
            "modal".into(),
            owned_value(true).map_err(|_| Error::new(ErrorKind::InvalidResponse))?,
        ),
    ]))
}

fn prepare_options(token: &str, formats: &[&str]) -> Result<HashMap<String, OwnedValue>, Error> {
    let mut options = base_options(token)?;
    options.insert(
        "accept_label".into(),
        owned_value("Print").map_err(|_| Error::new(ErrorKind::InvalidResponse))?,
    );
    insert_supported_formats(&mut options, formats)?;
    Ok(options)
}

fn submission_options(
    token: &str,
    submission: &rmac_print::PrintSubmission,
) -> Result<HashMap<String, OwnedValue>, Error> {
    let mut options = base_options(token)?;
    options.insert(
        "token".into(),
        owned_value(submission.token).map_err(|_| Error::new(ErrorKind::InvalidResponse))?,
    );
    insert_supported_formats(&mut options, submission.supported_output_file_formats())?;
    Ok(options)
}

fn insert_supported_formats(
    options: &mut HashMap<String, OwnedValue>,
    formats: &[&str],
) -> Result<(), Error> {
    options.insert(
        "supported_output_file_formats".into(),
        owned_value(
            formats
                .iter()
                .map(|format| (*format).to_string())
                .collect::<Vec<_>>(),
        )
        .map_err(|_| Error::new(ErrorKind::InvalidResponse))?,
    );
    Ok(())
}

fn owned_value<T>(value: T) -> Result<OwnedValue, zbus::zvariant::Error>
where
    T: zbus::zvariant::Type + Into<Value<'static>>,
{
    OwnedValue::try_from(Value::new(value))
}

fn decode_prepared(
    mut values: HashMap<String, OwnedValue>,
) -> Result<(u32, PageDescription, Option<String>), Error> {
    let token = values
        .remove("token")
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| Error::new(ErrorKind::InvalidResponse))?;
    let settings = values
        .remove("settings")
        .and_then(|value| HashMap::<String, OwnedValue>::try_from(value).ok())
        .ok_or_else(|| Error::new(ErrorKind::InvalidResponse))?;
    let page = values
        .remove("page-setup")
        .and_then(|value| HashMap::<String, OwnedValue>::try_from(value).ok())
        .ok_or_else(|| Error::new(ErrorKind::InvalidResponse))?;
    let description = PageDescription {
        width_mm: Some(required_number(&page, "Width")?),
        height_mm: Some(required_number(&page, "Height")?),
        margin_top_mm: Some(required_number(&page, "MarginTop")?),
        margin_right_mm: Some(required_number(&page, "MarginRight")?),
        margin_bottom_mm: Some(required_number(&page, "MarginBottom")?),
        margin_left_mm: Some(required_number(&page, "MarginLeft")?),
        orientation: match string(&page, "Orientation") {
            Some(value) => Some(
                PageOrientation::from_portal(&value)
                    .ok_or_else(|| Error::new(ErrorKind::InvalidResponse))?,
            ),
            None => None,
        },
    };
    let format = string(&settings, "output-file-format");
    Ok((token, description, format))
}

fn number(values: &HashMap<String, OwnedValue>, key: &str) -> Option<f64> {
    values.get(key).and_then(|value| f64::try_from(value).ok())
}

fn required_number(values: &HashMap<String, OwnedValue>, key: &str) -> Result<f64, Error> {
    number(values, key).ok_or_else(|| Error::new(ErrorKind::InvalidResponse))
}

fn string(values: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    values
        .get(key)
        .and_then(|value| <&str>::try_from(value).ok())
        .map(str::to_string)
}

fn printable_descriptor(bytes: &[u8]) -> std::io::Result<std::fs::File> {
    #[cfg(target_os = "linux")]
    {
        use rustix::fs::{MemfdFlags, SealFlags};

        let descriptor = rustix::fs::memfd_create(
            "rmac-text-editor-print",
            MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
        )?;
        let mut file = std::fs::File::from(descriptor);
        file.write_all(bytes)?;
        file.flush()?;
        file.seek(std::io::SeekFrom::Start(0))?;
        rustix::fs::fcntl_add_seals(
            &file,
            SealFlags::SHRINK | SealFlags::GROW | SealFlags::WRITE | SealFlags::SEAL,
        )?;
        Ok(file)
    }

    #[cfg(not(target_os = "linux"))]
    {
        use std::os::unix::fs::OpenOptionsExt as _;

        static NEXT_DESCRIPTOR: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "rmac-print-test-{}-{}",
            std::process::id(),
            NEXT_DESCRIPTOR.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        std::fs::remove_file(path)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.seek(std::io::SeekFrom::Start(0))?;
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn print_request(document_generation: u64) -> PrintDocument {
        let pointer = std::ptr::NonNull::<u8>::dangling().cast();
        PrintDocument {
            window: RawWindowHandle::Wayland(raw_window_handle::WaylandWindowHandle::new(pointer)),
            display: RawDisplayHandle::Wayland(raw_window_handle::WaylandDisplayHandle::new(
                pointer,
            )),
            window_generation: 7,
            document_generation,
            current_document_generation: Arc::new(AtomicU64::new(document_generation)),
            title: "private-title.txt".into(),
            text: "private document body".into(),
        }
    }

    fn value<T>(value: T) -> OwnedValue
    where
        T: zbus::zvariant::Type + Into<Value<'static>>,
    {
        owned_value(value).unwrap()
    }

    #[test]
    fn prepared_response_maps_exact_page_geometry_and_pdf() {
        let settings: HashMap<String, OwnedValue> =
            HashMap::from([("output-file-format".into(), value("PDF".to_string()))]);
        let page: HashMap<String, OwnedValue> = HashMap::from([
            ("Width".into(), value(210.0_f64)),
            ("Height".into(), value(297.0_f64)),
            ("MarginTop".into(), value(12.0_f64)),
            ("MarginRight".into(), value(13.0_f64)),
            ("MarginBottom".into(), value(14.0_f64)),
            ("MarginLeft".into(), value(15.0_f64)),
            ("Orientation".into(), value("landscape".to_string())),
        ]);
        let values: HashMap<String, OwnedValue> = HashMap::from([
            ("token".into(), value(42_u32)),
            ("settings".into(), OwnedValue::from(settings)),
            ("page-setup".into(), OwnedValue::from(page)),
        ]);

        let (token, page, format) = decode_prepared(values).unwrap();

        assert_eq!(token, 42);
        assert_eq!(page.width_mm, Some(210.0));
        assert_eq!(page.height_mm, Some(297.0));
        assert_eq!(page.margin_top_mm, Some(12.0));
        assert_eq!(page.margin_right_mm, Some(13.0));
        assert_eq!(page.margin_bottom_mm, Some(14.0));
        assert_eq!(page.margin_left_mm, Some(15.0));
        assert_eq!(page.orientation, Some(PageOrientation::Landscape));
        assert_eq!(format.as_deref(), Some("PDF"));
    }

    #[test]
    fn missing_required_prepare_fields_fail_closed() {
        assert!(decode_prepared(HashMap::new()).is_err());
        let only_token = HashMap::from([("token".into(), value(1_u32))]);
        assert!(decode_prepared(only_token).is_err());

        let incomplete_page: HashMap<String, OwnedValue> = HashMap::new();
        let incomplete = HashMap::from([
            ("token".into(), value(1_u32)),
            (
                "settings".into(),
                OwnedValue::from(HashMap::<String, OwnedValue>::new()),
            ),
            ("page-setup".into(), OwnedValue::from(incomplete_page)),
        ]);
        assert!(decode_prepared(incomplete).is_err());
    }

    #[test]
    fn request_debug_is_private_and_generation_changes_fail_closed() {
        let request = print_request(12);
        let debug = format!("{request:?}");
        assert!(!debug.contains("private-title"));
        assert!(!debug.contains("private document body"));
        assert!(!debug.contains("0x"));
        assert!(debug.contains("<private>"));

        let identity = PrintIdentity::new("wayland:~rmac-window-7", 7, 12).unwrap();
        assert!(require_current(&request, &identity, "wayland:~rmac-window-7").is_ok());
        request
            .current_document_generation
            .store(13, Ordering::Release);
        let error = require_current(&request, &identity, "wayland:~rmac-window-7").unwrap_err();
        assert!(error.is_stale_document());
    }

    #[test]
    fn options_bind_modal_pdf_only_prepare_and_submission_requests() {
        let mut prepare = prepare_options("rmac_prepare_1_2", &["pdf"]).unwrap();

        assert_eq!(
            prepare
                .get("handle_token")
                .and_then(|value| <&str>::try_from(value).ok()),
            Some("rmac_prepare_1_2")
        );
        assert_eq!(
            prepare
                .get("modal")
                .and_then(|value| bool::try_from(value).ok()),
            Some(true)
        );
        assert_eq!(
            prepare
                .get("accept_label")
                .and_then(|value| <&str>::try_from(value).ok()),
            Some("Print")
        );
        assert_eq!(
            prepare
                .remove("supported_output_file_formats")
                .and_then(|value| Vec::<String>::try_from(value).ok()),
            Some(vec!["pdf".to_string()])
        );

        let identity = PrintIdentity::new("wayland:~rmac-window-1", 1, 1).unwrap();
        let mut transaction = PortalPrintTransaction::new(identity.clone(), "Document").unwrap();
        let layout = transaction
            .prepared(&identity, 44, PageDescription::default(), Some("PDF"))
            .unwrap();
        transaction
            .rendered(&identity, layout, OutputFormat::Pdf)
            .unwrap();
        let submission = transaction.submission(&identity).unwrap();
        let mut submit = submission_options("rmac_submit_1_3", &submission).unwrap();

        assert_eq!(
            submit
                .remove("token")
                .and_then(|value| u32::try_from(value).ok()),
            Some(44)
        );
        assert_eq!(
            submit
                .remove("supported_output_file_formats")
                .and_then(|value| Vec::<String>::try_from(value).ok()),
            Some(vec!["pdf".to_string()])
        );
    }

    #[test]
    fn printable_pdf_descriptor_is_readable_and_sealed() {
        use std::io::Read as _;

        let mut descriptor = printable_descriptor(b"%PDF-1.7\n%%EOF\n").unwrap();
        let mut bytes = Vec::new();
        descriptor.read_to_end(&mut bytes).unwrap();

        assert_eq!(bytes, b"%PDF-1.7\n%%EOF\n");
        #[cfg(target_os = "linux")]
        {
            let seals = rustix::fs::fcntl_get_seals(&descriptor).unwrap();
            assert!(seals.contains(rustix::fs::SealFlags::WRITE));
            assert!(seals.contains(rustix::fs::SealFlags::SEAL));
        }
    }
}
