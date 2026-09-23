use std::fmt;

/// MPRIS bus names all share this prefix; the remainder names the player.
pub const BUS_PREFIX: &str = "org.mpris.MediaPlayer2.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
}

impl PlaybackStatus {
    /// Unknown values are treated as stopped, which never claims playback.
    pub fn parse(value: &str) -> Self {
        match value {
            "Playing" => Self::Playing,
            "Paused" => Self::Paused,
            _ => Self::Stopped,
        }
    }
}

/// One media player as reported by its MPRIS interface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Player {
    pub bus_name: String,
    pub identity: String,
    pub status: PlaybackStatus,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub art_url: Option<String>,
    pub can_control: bool,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_go_next: bool,
    pub can_go_previous: bool,
}

impl Player {
    pub fn has_track(&self) -> bool {
        self.title.as_deref().is_some_and(|title| !title.is_empty())
    }
}

/// Choose the player that Now Playing and the media keys act on, the way
/// macOS does: whatever is playing wins, preferring the player the user
/// controlled last; otherwise the last-controlled player while it still has a
/// track; otherwise any paused player with a track.
pub fn select_active<'a>(players: &'a [Player], last_active: Option<&str>) -> Option<&'a Player> {
    let controllable = || players.iter().filter(|player| player.can_control);
    let is_last = |player: &&Player| Some(player.bus_name.as_str()) == last_active;
    let playing = |player: &&Player| player.status == PlaybackStatus::Playing;

    controllable()
        .filter(playing)
        .find(is_last)
        .or_else(|| controllable().find(playing))
        .or_else(|| {
            controllable()
                .filter(|player| player.status != PlaybackStatus::Stopped || player.has_track())
                .find(is_last)
        })
        .or_else(|| {
            controllable()
                .find(|player| player.status == PlaybackStatus::Paused && player.has_track())
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    PlayPause,
    Play,
    Pause,
    Stop,
    Next,
    Previous,
}

impl Command {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "play-pause" => Self::PlayPause,
            "play" => Self::Play,
            "pause" => Self::Pause,
            "stop" => Self::Stop,
            "next" => Self::Next,
            "previous" => Self::Previous,
            _ => return None,
        })
    }

    /// The MPRIS method name for this command.
    pub fn method(self) -> &'static str {
        match self {
            Self::PlayPause => "PlayPause",
            Self::Play => "Play",
            Self::Pause => "Pause",
            Self::Stop => "Stop",
            Self::Next => "Next",
            Self::Previous => "Previous",
        }
    }

    /// Whether the player advertises support for this command.
    pub fn supported_by(self, player: &Player) -> bool {
        match self {
            Self::PlayPause => player.can_play || player.can_pause,
            Self::Play => player.can_play,
            Self::Pause => player.can_pause,
            Self::Stop => player.can_control,
            Self::Next => player.can_go_next,
            Self::Previous => player.can_go_previous,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    /// No session bus, or the platform has no MPRIS.
    Unavailable,
    /// No player can take the command.
    NoPlayer,
    /// The player rejected or failed the command.
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
}

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Error {}
