//! Authoritative system OSD commands and bounded local presentation transport.

use serde::{Deserialize, Serialize};
use std::fmt;

#[cfg(target_os = "linux")]
mod linux;

const PRESENTATION_VERSION: u8 = 1;
const MAX_TITLE_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    VolumeUp,
    VolumeDown,
    ToggleOutputMute,
    ToggleInputMute,
    BrightnessUp,
    BrightnessDown,
    ShowVolume,
    ShowBrightness,
}

impl Command {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "volume-up" => Some(Self::VolumeUp),
            "volume-down" => Some(Self::VolumeDown),
            "toggle-output-mute" => Some(Self::ToggleOutputMute),
            "toggle-input-mute" => Some(Self::ToggleInputMute),
            "brightness-up" => Some(Self::BrightnessUp),
            "brightness-down" => Some(Self::BrightnessDown),
            "show-volume" => Some(Self::ShowVolume),
            "show-brightness" => Some(Self::ShowBrightness),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Output,
    Input,
    Display,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Presentation {
    version: u8,
    pub kind: Kind,
    pub title: String,
    pub level: u8,
    pub muted: bool,
}

impl Presentation {
    pub fn new(
        kind: Kind,
        title: impl Into<String>,
        level: u8,
        muted: bool,
    ) -> Result<Self, Error> {
        let presentation = Self {
            version: PRESENTATION_VERSION,
            kind,
            title: title.into(),
            level,
            muted,
        };
        presentation.validate()?;
        Ok(presentation)
    }

    pub fn visible_level(&self) -> u8 {
        if self.muted {
            0
        } else {
            self.level
        }
    }

    pub fn accessible_label(&self) -> String {
        match (self.kind, self.muted) {
            (Kind::Output, true) => format!("{} muted", self.title),
            (Kind::Input, true) => format!("{} muted", self.title),
            (Kind::Output, false) => format!("{} volume {} percent", self.title, self.level),
            (Kind::Input, false) => format!("{} level {} percent", self.title, self.level),
            (Kind::Display, _) => format!("Display brightness {} percent", self.level),
        }
    }

    fn validate(&self) -> Result<(), Error> {
        if self.version != PRESENTATION_VERSION
            || self.level > 100
            || (self.kind == Kind::Display && self.muted)
            || self.title.is_empty()
            || self.title.len() > MAX_TITLE_BYTES
            || self.title.trim() != self.title
            || self.title.chars().any(char::is_control)
        {
            return Err(Error::new(Operation::ValidatePresentation));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    ParseCommand,
    ReadAudio,
    ChangeAudio,
    ReadBrightness,
    ResolveSession,
    ChangeBrightness,
    ValidatePresentation,
    ResolveRuntime,
    BindTransport,
    ReadTransport,
    SendTransport,
}

impl fmt::Display for Operation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ParseCommand => "parse the OSD command",
            Self::ReadAudio => "read audio state",
            Self::ChangeAudio => "change audio state",
            Self::ReadBrightness => "read display brightness",
            Self::ResolveSession => "resolve the active local session",
            Self::ChangeBrightness => "change display brightness",
            Self::ValidatePresentation => "validate OSD presentation",
            Self::ResolveRuntime => "resolve the OSD runtime directory",
            Self::BindTransport => "bind the OSD endpoint",
            Self::ReadTransport => "read an OSD update",
            Self::SendTransport => "send an OSD update",
        })
    }
}

#[derive(Debug)]
pub struct Error {
    pub operation: Operation,
}

impl Error {
    fn new(operation: Operation) -> Self {
        Self { operation }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not {}", self.operation)
    }
}

impl std::error::Error for Error {}

pub fn execute(command: Command) -> Result<Presentation, Error> {
    #[cfg(target_os = "linux")]
    {
        linux::execute(command)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = command;
        Err(Error::new(Operation::ParseCommand))
    }
}

/// The preferred backlight's level in percent; an error when the machine has
/// no backlight Control Center can drive.
pub fn brightness() -> Result<u8, Error> {
    #[cfg(target_os = "linux")]
    {
        linux::brightness()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(Error::new(Operation::ReadBrightness))
    }
}

/// Set the preferred backlight to `percentage` through logind and return the
/// level read back from sysfs.
pub fn set_brightness(percentage: u8) -> Result<u8, Error> {
    #[cfg(target_os = "linux")]
    {
        linux::set_brightness_percentage(percentage)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = percentage;
        Err(Error::new(Operation::ChangeBrightness))
    }
}

#[cfg(target_os = "linux")]
pub use linux::{send, Listener};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_an_exact_allow_list() {
        assert_eq!(Command::parse("volume-up"), Some(Command::VolumeUp));
        assert_eq!(
            Command::parse("show-brightness"),
            Some(Command::ShowBrightness)
        );
        assert_eq!(Command::parse("volume-up;shutdown"), None);
        assert_eq!(Command::parse(""), None);
    }

    #[test]
    fn presentations_are_bounded_and_semantic() {
        let muted = Presentation::new(Kind::Output, "Built-in Speakers", 61, true).unwrap();
        assert_eq!(muted.visible_level(), 0);
        assert_eq!(muted.accessible_label(), "Built-in Speakers muted");
        assert!(Presentation::new(Kind::Display, "Display", 42, true).is_err());
        assert!(Presentation::new(Kind::Display, "Display", 101, false).is_err());
        assert!(Presentation::new(Kind::Output, " bad", 42, false).is_err());
        assert!(Presentation::new(Kind::Output, "x".repeat(129), 42, false).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn wire_round_trip_revalidates_the_payload() {
        let presentation = Presentation::new(Kind::Display, "Display", 73, false).unwrap();
        let bytes = serde_json::to_vec(&presentation).unwrap();
        assert_eq!(linux::decode(&bytes).unwrap(), presentation);

        let invalid =
            br#"{"version":1,"kind":"display","title":"Display","level":255,"muted":false}"#;
        assert!(linux::decode(invalid).is_err());
    }
}
