//! Typed, bounded model of one `org.freedesktop.impl.portal.FileChooser`
//! request. The D-Bus adapter decodes wire options into [`RawOptions`];
//! everything here is plain Rust so it is unit-tested without a bus.

use std::fmt;
use std::path::PathBuf;

use crate::parent::ParentWindow;

pub const MAX_TEXT_BYTES: usize = 1024;
pub const MAX_FILTERS: usize = 64;
pub const MAX_RULES_PER_FILTER: usize = 256;
pub const MAX_CHOICES: usize = 16;
pub const MAX_CHOICE_OPTIONS: usize = 64;
pub const MAX_SAVE_FILES: usize = 1024;
pub const MAX_PATH_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Open,
    Save,
    SaveFiles,
}

/// One pattern of a portal filter: `(0, glob)` or `(1, mime type)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rule {
    Glob(String),
    Mime(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    pub name: String,
    pub rules: Vec<Rule>,
}

/// An application-defined choice: a pop-up when it has options, a checkbox
/// (`"true"`/`"false"`) when it has none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub label: String,
    pub options: Vec<(String, String)>,
    pub selected: String,
}

impl Choice {
    pub fn is_checkbox(&self) -> bool {
        self.options.is_empty()
    }

    pub fn checked(&self) -> bool {
        self.selected == "true"
    }
}

/// Wire-shaped options exactly as the portal sends them.
pub type WireFilter = (String, Vec<(u32, String)>);
pub type WireChoice = (String, String, Vec<(String, String)>, String);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawOptions {
    pub accept_label: Option<String>,
    pub modal: Option<bool>,
    pub multiple: Option<bool>,
    pub directory: Option<bool>,
    pub filters: Option<Vec<WireFilter>>,
    pub current_filter: Option<WireFilter>,
    pub choices: Option<Vec<WireChoice>>,
    pub current_name: Option<String>,
    pub current_folder: Option<Vec<u8>>,
    pub current_file: Option<Vec<u8>>,
    pub files: Option<Vec<Vec<u8>>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    pub mode: Mode,
    pub app_id: String,
    pub parent: ParentWindow,
    pub title: String,
    pub accept_label: Option<String>,
    pub modal: bool,
    pub multiple: bool,
    pub directory: bool,
    pub filters: Vec<Filter>,
    pub current_filter: Option<usize>,
    pub choices: Vec<Choice>,
    pub current_name: Option<String>,
    pub current_folder: Option<PathBuf>,
    pub current_file: Option<PathBuf>,
    /// SaveFiles: the file names the application wants to write.
    pub files: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidRequest(pub &'static str);

impl fmt::Display for InvalidRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid FileChooser request: {}", self.0)
    }
}

impl std::error::Error for InvalidRequest {}

fn text(value: String, field: &'static str) -> Result<String, InvalidRequest> {
    if value.len() > MAX_TEXT_BYTES || value.contains('\0') {
        Err(InvalidRequest(field))
    } else {
        Ok(value)
    }
}

/// Decode a portal `ay` path: trailing NULs are stripped, the rest must be a
/// bounded absolute path without interior NULs.
pub fn wire_path(bytes: Vec<u8>, field: &'static str) -> Result<Option<PathBuf>, InvalidRequest> {
    use std::os::unix::ffi::OsStringExt as _;
    let mut bytes = bytes;
    while bytes.last() == Some(&0) {
        bytes.pop();
    }
    if bytes.is_empty() {
        return Ok(None);
    }
    if bytes.len() > MAX_PATH_BYTES || bytes.contains(&0) || bytes[0] != b'/' {
        return Err(InvalidRequest(field));
    }
    Ok(Some(PathBuf::from(std::ffi::OsString::from_vec(bytes))))
}

fn filter(wire: WireFilter) -> Result<Filter, InvalidRequest> {
    let (name, patterns) = wire;
    if patterns.len() > MAX_RULES_PER_FILTER {
        return Err(InvalidRequest("filters"));
    }
    let rules = patterns
        .into_iter()
        .map(|(kind, pattern)| {
            let pattern = text(pattern, "filters")?;
            match kind {
                0 => Ok(Rule::Glob(pattern)),
                1 => Ok(Rule::Mime(pattern)),
                _ => Err(InvalidRequest("filters")),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Filter {
        name: text(name, "filters")?,
        rules,
    })
}

fn choice(wire: WireChoice) -> Result<Choice, InvalidRequest> {
    let (id, label, options, selected) = wire;
    if options.len() > MAX_CHOICE_OPTIONS {
        return Err(InvalidRequest("choices"));
    }
    let options = options
        .into_iter()
        .map(|(key, label)| Ok((text(key, "choices")?, text(label, "choices")?)))
        .collect::<Result<Vec<_>, InvalidRequest>>()?;
    let mut selected = text(selected, "choices")?;
    if options.is_empty() {
        if selected != "true" {
            selected = "false".to_owned();
        }
    } else if !options.iter().any(|(key, _)| *key == selected) {
        selected = options[0].0.clone();
    }
    Ok(Choice {
        id: text(id, "choices")?,
        label: text(label, "choices")?,
        options,
        selected,
    })
}

/// A file name the panel may write: one non-empty path component.
pub fn valid_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\0')
}

impl Request {
    pub fn from_wire(
        mode: Mode,
        app_id: String,
        parent_window: &str,
        title: String,
        options: RawOptions,
    ) -> Result<Self, InvalidRequest> {
        let mut filters = options
            .filters
            .unwrap_or_default()
            .into_iter()
            .take(MAX_FILTERS + 1)
            .map(filter)
            .collect::<Result<Vec<_>, _>>()?;
        if filters.len() > MAX_FILTERS {
            return Err(InvalidRequest("filters"));
        }
        let current_filter = match options.current_filter.map(filter).transpose()? {
            None => (!filters.is_empty()).then_some(0),
            Some(current) => match filters.iter().position(|known| *known == current) {
                Some(index) => Some(index),
                // The portal spec lets current_filter stand alone when the
                // list is empty; it then becomes the only filter.
                None if filters.is_empty() => {
                    filters.push(current);
                    Some(0)
                }
                None => Some(0),
            },
        };
        let choices = options.choices.unwrap_or_default();
        if choices.len() > MAX_CHOICES {
            return Err(InvalidRequest("choices"));
        }
        let choices = choices
            .into_iter()
            .map(choice)
            .collect::<Result<Vec<_>, _>>()?;
        let current_name = options
            .current_name
            .map(|name| text(name, "current_name"))
            .transpose()?
            .map(|name| name.replace('/', "-"))
            .filter(|name| !name.is_empty());
        let current_folder = options
            .current_folder
            .map(|bytes| wire_path(bytes, "current_folder"))
            .transpose()?
            .flatten();
        let current_file = options
            .current_file
            .map(|bytes| wire_path(bytes, "current_file"))
            .transpose()?
            .flatten();
        let wire_files = options.files.unwrap_or_default();
        if wire_files.len() > MAX_SAVE_FILES {
            return Err(InvalidRequest("files"));
        }
        let mut files = Vec::with_capacity(wire_files.len());
        for mut bytes in wire_files {
            while bytes.last() == Some(&0) {
                bytes.pop();
            }
            // Applications send full or bare names; only the final component
            // is ever used, inside the folder the user chooses.
            let name = String::from_utf8_lossy(&bytes);
            let name = name.rsplit('/').next().unwrap_or_default().to_owned();
            if !valid_file_name(&name) {
                return Err(InvalidRequest("files"));
            }
            files.push(name);
        }
        let directory = mode == Mode::Open && options.directory.unwrap_or(false);
        Ok(Self {
            mode,
            app_id: text(app_id, "app_id")?,
            parent: ParentWindow::parse(parent_window),
            title: text(title, "title")?,
            accept_label: options
                .accept_label
                .map(|label| text(label, "accept_label"))
                .transpose()?
                .map(|label| label.replace('_', ""))
                .filter(|label| !label.is_empty()),
            modal: options.modal.unwrap_or(true),
            multiple: mode == Mode::Open && options.multiple.unwrap_or(false),
            directory,
            filters,
            current_filter,
            choices,
            current_name,
            current_folder,
            current_file,
            files,
        })
    }

    /// The default button's label, macOS wording unless the app supplied one
    /// (GTK mnemonic underscores are removed).
    pub fn accept_label(&self) -> String {
        if let Some(label) = &self.accept_label {
            return label.clone();
        }
        match self.mode {
            Mode::Open => "Open".to_owned(),
            Mode::Save | Mode::SaveFiles => "Save".to_owned(),
        }
    }

    /// The initial Save As text: an explicit name, else the current file's.
    pub fn initial_name(&self) -> String {
        self.current_name
            .clone()
            .or_else(|| {
                self.current_file
                    .as_ref()
                    .and_then(|file| file.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "Untitled".to_owned())
    }

    /// The folder the panel starts in, before falling back to a default.
    pub fn initial_folder(&self) -> Option<PathBuf> {
        self.current_folder.clone().or_else(|| {
            self.current_file
                .as_ref()
                .and_then(|file| file.parent())
                .map(PathBuf::from)
        })
    }
}
