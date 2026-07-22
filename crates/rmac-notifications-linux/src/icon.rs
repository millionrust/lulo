//! Scale-aware, bounded notification icon preparation for the E2 renderer.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use rmac_notifications::{AppId, NotificationId};

use crate::media::{Icon, IconFormat};

pub const MIN_LOGICAL_ICON_EDGE: u16 = 16;
pub const MAX_LOGICAL_ICON_EDGE: u16 = 128;
pub const MIN_OUTPUT_SCALE: f64 = 0.5;
pub const MAX_OUTPUT_SCALE: f64 = 4.0;
pub const DEFAULT_ICON_CACHE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_APPLICATION_ICONS: usize = 4_096;
const MAX_MEMORY_CACHE_ENTRIES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Request {
    logical_edge: u16,
    pixel_edge: u32,
    decode: rmac_icon::DecodeRequest,
}

impl Request {
    pub fn new(logical_edge: u16, output_scale: f64) -> Result<Self, Error> {
        if !(MIN_LOGICAL_ICON_EDGE..=MAX_LOGICAL_ICON_EDGE).contains(&logical_edge)
            || !output_scale.is_finite()
            || !(MIN_OUTPUT_SCALE..=MAX_OUTPUT_SCALE).contains(&output_scale)
        {
            return Err(Error::InvalidRequest);
        }
        let pixel_edge = (f64::from(logical_edge) * output_scale).ceil() as u32;
        if pixel_edge == 0 || pixel_edge > rmac_icon::MAX_ICON_EDGE {
            return Err(Error::InvalidRequest);
        }
        let decode =
            rmac_icon::DecodeRequest::new(pixel_edge).map_err(|_| Error::InvalidRequest)?;
        Ok(Self {
            logical_edge,
            pixel_edge,
            decode,
        })
    }

    pub fn logical_edge(self) -> u16 {
        self.logical_edge
    }

    pub fn pixel_edge(self) -> u32 {
        self.pixel_edge
    }

    fn decode_request(self) -> rmac_icon::DecodeRequest {
        self.decode
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Origin {
    PortalFile,
    PortalTheme,
    Application,
}

#[derive(Clone, Eq, PartialEq)]
pub struct ReadyIcon {
    logical_edge: u16,
    origin: Origin,
    decoded: Arc<rmac_icon::DecodedIcon>,
}

impl ReadyIcon {
    pub fn logical_edge(&self) -> u16 {
        self.logical_edge
    }

    pub fn pixel_edge(&self) -> u32 {
        self.decoded.edge()
    }

    pub fn origin(&self) -> Origin {
        self.origin
    }

    pub fn format(&self) -> rmac_icon::SourceFormat {
        self.decoded.format()
    }

    pub fn rgba(&self) -> &Arc<[u8]> {
        self.decoded.rgba()
    }
}

impl fmt::Debug for ReadyIcon {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReadyIcon")
            .field("logical_edge", &self.logical_edge)
            .field("origin", &self.origin)
            .field("decoded", &self.decoded)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FallbackReason {
    NoSource,
    NotFound,
    InvalidApplication,
    FormatMismatch,
    Decode(rmac_icon::ErrorKind),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Outcome {
    Ready(Arc<ReadyIcon>),
    Fallback(FallbackReason),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidRequest,
    TooManyApplications,
    InvalidApplication,
    InvalidPath,
    DuplicateApplication,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "notification icon preparation failed ({self:?})")
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Default, Eq, PartialEq)]
pub struct ApplicationIcons {
    paths: BTreeMap<String, PathBuf>,
}

impl ApplicationIcons {
    pub fn new(entries: impl IntoIterator<Item = (String, PathBuf)>) -> Result<Self, Error> {
        let entries = entries.into_iter().collect::<Vec<_>>();
        if entries.len() > MAX_APPLICATION_ICONS {
            return Err(Error::TooManyApplications);
        }
        let mut exact = BTreeMap::new();
        for (app_id, path) in entries {
            AppId::parse(app_id.clone()).map_err(|_| Error::InvalidApplication)?;
            validate_path(&path)?;
            if exact.insert(app_id, path).is_some() {
                return Err(Error::DuplicateApplication);
            }
        }
        let aliases = exact
            .iter()
            .filter_map(|(app_id, path)| {
                app_id
                    .strip_suffix(".desktop")
                    .map(|alias| (alias.to_owned(), path.clone()))
            })
            .collect::<Vec<_>>();
        for (alias, path) in aliases {
            exact.entry(alias).or_insert(path);
        }
        Ok(Self { paths: exact })
    }

    pub fn from_applications(applications: &[rmac_apps::Application]) -> Result<Self, Error> {
        Self::new(applications.iter().filter_map(|application| {
            application
                .icon
                .as_ref()
                .map(|path| (application.id.clone(), path.clone()))
        }))
    }

    fn get(&self, app_id: &str) -> Option<&PathBuf> {
        self.paths.get(app_id)
    }

    fn len(&self) -> usize {
        self.paths.len()
    }
}

impl fmt::Debug for ApplicationIcons {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApplicationIcons")
            .field("entries", &self.paths.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Stats {
    pub memory_entries: usize,
    pub memory_bytes: usize,
    pub memory_decodes: u64,
    pub file: rmac_icon::CacheStats,
}

struct MemoryEntry {
    notification: NotificationId,
    edge: u32,
    format: IconFormat,
    source: Weak<[u8]>,
    decoded: Arc<rmac_icon::DecodedIcon>,
    last_used: u64,
}

struct State {
    themes: rmac_apps::ThemedIconResolver,
    applications: ApplicationIcons,
    files: rmac_icon::Cache,
    memory: Vec<MemoryEntry>,
    memory_bytes: usize,
    memory_decodes: u64,
    sequence: u64,
    byte_budget: usize,
}

/// Serialized icon resolver/decoder. Portal bytes, theme paths, application
/// paths, decoded pixels, and cache keys never appear in diagnostics.
pub struct Renderer {
    state: Mutex<State>,
}

impl Renderer {
    pub fn new(applications: ApplicationIcons) -> Self {
        Self::with_budget(applications, DEFAULT_ICON_CACHE_BYTES)
    }

    pub fn with_budget(applications: ApplicationIcons, byte_budget: usize) -> Self {
        Self {
            state: Mutex::new(State {
                themes: rmac_apps::ThemedIconResolver::current(),
                applications,
                files: rmac_icon::Cache::new(byte_budget),
                memory: Vec::new(),
                memory_bytes: 0,
                memory_decodes: 0,
                sequence: 0,
                byte_budget,
            }),
        }
    }

    /// Resolves and decodes synchronously. The eventual surface host must call
    /// this method from its bounded icon worker, never the render thread.
    pub fn render(
        &self,
        notification: NotificationId,
        app_id: &str,
        portal: Option<&Icon>,
        request: Request,
    ) -> Outcome {
        if AppId::parse(app_id.to_owned()).is_err() {
            return Outcome::Fallback(FallbackReason::InvalidApplication);
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut last_failure = None;
        if let Some(portal) = portal {
            match portal {
                Icon::File { format, bytes } => {
                    match render_memory(&mut state, notification, *format, bytes, request) {
                        Ok(ready) => return Outcome::Ready(ready),
                        Err(reason) => last_failure = Some(reason),
                    }
                }
                Icon::Themed(names) => {
                    for name in names {
                        let Some(path) = state.themes.resolve(name, request.pixel_edge) else {
                            continue;
                        };
                        match state.files.get_or_decode(&path, request.decode_request()) {
                            Ok(decoded) => {
                                return Outcome::Ready(Arc::new(ReadyIcon {
                                    logical_edge: request.logical_edge,
                                    origin: Origin::PortalTheme,
                                    decoded,
                                }));
                            }
                            Err(error) => last_failure = Some(FallbackReason::Decode(error.kind())),
                        }
                    }
                    last_failure.get_or_insert(FallbackReason::NotFound);
                }
            }
        }
        if let Some(path) = state.applications.get(app_id).cloned() {
            match state.files.get_or_decode(&path, request.decode_request()) {
                Ok(decoded) => {
                    return Outcome::Ready(Arc::new(ReadyIcon {
                        logical_edge: request.logical_edge,
                        origin: Origin::Application,
                        decoded,
                    }));
                }
                Err(error) => last_failure = Some(FallbackReason::Decode(error.kind())),
            }
        }
        Outcome::Fallback(last_failure.unwrap_or(FallbackReason::NoSource))
    }

    pub fn replace_applications(&self, applications: ApplicationIcons) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.applications = applications;
        state.files.clear();
    }

    pub fn refresh_theme(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.themes = rmac_apps::ThemedIconResolver::current();
        state.files.clear();
    }

    pub fn retain_notifications(&self, ids: &BTreeSet<NotificationId>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state
            .memory
            .retain(|entry| ids.contains(&entry.notification));
        state.memory_bytes = state
            .memory
            .iter()
            .map(|entry| entry.decoded.rgba().len())
            .sum();
    }

    pub fn stats(&self) -> Stats {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Stats {
            memory_entries: state.memory.len(),
            memory_bytes: state.memory_bytes,
            memory_decodes: state.memory_decodes,
            file: state.files.stats(),
        }
    }
}

impl fmt::Debug for Renderer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        formatter
            .debug_struct("Renderer")
            .field("applications", &state.applications.len())
            .field(
                "stats",
                &Stats {
                    memory_entries: state.memory.len(),
                    memory_bytes: state.memory_bytes,
                    memory_decodes: state.memory_decodes,
                    file: state.files.stats(),
                },
            )
            .finish()
    }
}

fn render_memory(
    state: &mut State,
    notification: NotificationId,
    format: IconFormat,
    bytes: &Arc<[u8]>,
    request: Request,
) -> Result<Arc<ReadyIcon>, FallbackReason> {
    state.sequence = state.sequence.saturating_add(1);
    let sequence = state.sequence;
    state.memory.retain(|entry| {
        entry
            .source
            .upgrade()
            .is_some_and(|source| entry.notification != notification || Arc::ptr_eq(&source, bytes))
    });
    state.memory_bytes = state
        .memory
        .iter()
        .map(|entry| entry.decoded.rgba().len())
        .sum();
    if let Some(entry) = state.memory.iter_mut().find(|entry| {
        entry.notification == notification
            && entry.edge == request.pixel_edge
            && entry.format == format
            && entry
                .source
                .upgrade()
                .is_some_and(|source| Arc::ptr_eq(&source, bytes))
    }) {
        entry.last_used = sequence;
        return Ok(Arc::new(ReadyIcon {
            logical_edge: request.logical_edge,
            origin: Origin::PortalFile,
            decoded: entry.decoded.clone(),
        }));
    }
    let decoded = Arc::new(
        rmac_icon::decode_bytes(bytes, request.decode_request())
            .map_err(|error| FallbackReason::Decode(error.kind()))?,
    );
    if decoded.format() != source_format(format) {
        return Err(FallbackReason::FormatMismatch);
    }
    let ready = Arc::new(ReadyIcon {
        logical_edge: request.logical_edge,
        origin: Origin::PortalFile,
        decoded: decoded.clone(),
    });
    state.memory_decodes = state.memory_decodes.saturating_add(1);
    state
        .memory
        .retain(|entry| entry.notification != notification || entry.edge != request.pixel_edge);
    state.memory_bytes = state
        .memory
        .iter()
        .map(|entry| entry.decoded.rgba().len())
        .sum();
    let bytes_len = ready.rgba().len();
    if state.byte_budget != 0 && bytes_len <= state.byte_budget {
        state.memory.push(MemoryEntry {
            notification,
            edge: request.pixel_edge,
            format,
            source: Arc::downgrade(bytes),
            decoded,
            last_used: sequence,
        });
        state.memory_bytes = state.memory_bytes.saturating_add(bytes_len);
        while state.memory_bytes > state.byte_budget
            || state.memory.len() > MAX_MEMORY_CACHE_ENTRIES
        {
            let Some(index) = state
                .memory
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(index, _)| index)
            else {
                break;
            };
            let removed = state.memory.remove(index);
            state.memory_bytes = state
                .memory_bytes
                .saturating_sub(removed.decoded.rgba().len());
        }
    }
    Ok(ready)
}

fn source_format(format: IconFormat) -> rmac_icon::SourceFormat {
    match format {
        IconFormat::Png => rmac_icon::SourceFormat::Png,
        IconFormat::Jpeg => rmac_icon::SourceFormat::Jpeg,
        IconFormat::Svg => rmac_icon::SourceFormat::Svg,
    }
}

fn validate_path(path: &Path) -> Result<(), Error> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(Error::InvalidPath);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::time::{SystemTime, UNIX_EPOCH};

    use image::ImageEncoder as _;

    use super::*;

    fn id(value: u32) -> NotificationId {
        NotificationId::from_protocol(value).unwrap()
    }

    fn png(rgba: [u8; 4]) -> Arc<[u8]> {
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&rgba, 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        bytes.into()
    }

    fn root(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "rmac-notification-icon-{label}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn request_maps_fractional_scale_to_exact_bounded_pixels() {
        let request = Request::new(40, 1.25).unwrap();
        assert_eq!(request.logical_edge(), 40);
        assert_eq!(request.pixel_edge(), 50);
        assert_eq!(Request::new(40, 1.251).unwrap().pixel_edge(), 51);
        for invalid in [f64::NAN, 0.49, 4.01] {
            assert_eq!(Request::new(40, invalid), Err(Error::InvalidRequest));
        }
        assert_eq!(Request::new(15, 1.0), Err(Error::InvalidRequest));
        assert_eq!(Request::new(129, 1.0), Err(Error::InvalidRequest));
    }

    #[test]
    fn portal_pixels_are_cached_by_exact_source_identity_and_replacement() {
        let renderer = Renderer::with_budget(ApplicationIcons::default(), 2 * 64 * 64 * 4);
        let request = Request::new(32, 2.0).unwrap();
        let first_bytes = png([255, 0, 0, 255]);
        let first = Icon::File {
            format: IconFormat::Png,
            bytes: first_bytes,
        };
        let Outcome::Ready(first_ready) =
            renderer.render(id(1), "org.example.App", Some(&first), request)
        else {
            panic!("validated portal icon must render");
        };
        let Outcome::Ready(again) =
            renderer.render(id(1), "org.example.App", Some(&first), request)
        else {
            panic!("cached portal icon must render");
        };
        assert!(Arc::ptr_eq(first_ready.rgba(), again.rgba()));
        assert_eq!(first_ready.logical_edge(), 32);
        assert_eq!(first_ready.pixel_edge(), 64);
        assert_eq!(first_ready.origin(), Origin::PortalFile);
        assert_eq!(renderer.stats().memory_decodes, 1);

        let same_pixels = Request::new(64, 1.0).unwrap();
        let Outcome::Ready(logically_larger) =
            renderer.render(id(1), "org.example.App", Some(&first), same_pixels)
        else {
            panic!("a second logical size must render");
        };
        assert_eq!(logically_larger.logical_edge(), 64);
        assert_eq!(logically_larger.pixel_edge(), 64);
        assert!(!Arc::ptr_eq(&first_ready, &logically_larger));
        assert_eq!(renderer.stats().memory_decodes, 1);

        let replacement = Icon::File {
            format: IconFormat::Png,
            bytes: png([0, 0, 255, 255]),
        };
        let Outcome::Ready(replaced) =
            renderer.render(id(1), "org.example.App", Some(&replacement), request)
        else {
            panic!("replacement portal icon must render");
        };
        assert!(!Arc::ptr_eq(&first_ready, &replaced));
        assert_ne!(first_ready.rgba(), replaced.rgba());
        assert_eq!(renderer.stats().memory_decodes, 2);
        assert_eq!(renderer.stats().memory_entries, 1);
    }

    #[test]
    fn exact_application_alias_fallback_is_private_and_live_file_aware() {
        let root = root("application");
        let path = root.join("private-application-icon.png");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(&png([10, 20, 30, 255]))
            .unwrap();
        let catalog =
            ApplicationIcons::new([("org.example.App.desktop".into(), path.clone())]).unwrap();
        let renderer = Renderer::new(catalog);
        let request = Request::new(40, 1.0).unwrap();
        let Outcome::Ready(ready) = renderer.render(id(1), "org.example.App", None, request) else {
            panic!("exact suffix alias must resolve");
        };
        assert_eq!(ready.origin(), Origin::Application);
        assert!(matches!(
            renderer.render(id(2), "example.App", None, request),
            Outcome::Fallback(FallbackReason::NoSource)
        ));
        let diagnostics = format!("{renderer:?}");
        assert!(!diagnostics.contains("private-application-icon"));
        assert!(!diagnostics.contains("org.example.App"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cache_budget_and_live_notification_retention_are_exact() {
        let one_icon = 32 * 32 * 4;
        let renderer = Renderer::with_budget(ApplicationIcons::default(), one_icon);
        let request = Request::new(32, 1.0).unwrap();
        for value in 1..=3 {
            let icon = Icon::File {
                format: IconFormat::Png,
                bytes: png([value as u8, 0, 0, 255]),
            };
            assert!(matches!(
                renderer.render(id(value), "org.example.App", Some(&icon), request),
                Outcome::Ready(_)
            ));
        }
        assert_eq!(renderer.stats().memory_entries, 1);
        assert_eq!(renderer.stats().memory_bytes, one_icon);
        renderer.retain_notifications(&BTreeSet::from([id(3)]));
        assert_eq!(renderer.stats().memory_entries, 1);
        renderer.retain_notifications(&BTreeSet::new());
        assert_eq!(renderer.stats().memory_entries, 0);
        assert_eq!(renderer.stats().memory_bytes, 0);
    }

    #[test]
    fn declared_format_mismatch_and_invalid_catalog_fail_closed() {
        let renderer = Renderer::new(ApplicationIcons::default());
        let icon = Icon::File {
            format: IconFormat::Jpeg,
            bytes: png([1, 2, 3, 255]),
        };
        assert_eq!(
            renderer.render(
                id(1),
                "org.example.App",
                Some(&icon),
                Request::new(32, 1.0).unwrap(),
            ),
            Outcome::Fallback(FallbackReason::FormatMismatch)
        );
        assert_eq!(
            ApplicationIcons::new([("bad\napp".into(), PathBuf::from("/private/icon.png"))]),
            Err(Error::InvalidApplication)
        );
        assert_eq!(
            ApplicationIcons::new([
                ("org.example.App".into(), PathBuf::from("relative/icon.png"),)
            ]),
            Err(Error::InvalidPath)
        );
    }
}
