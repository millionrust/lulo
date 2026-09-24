//! MPRIS 2 on the session bus, so rmac's media keys, the OSD and Control
//! Center's Now Playing (crates/rmac-media) see and control Media Player.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rmac_player::model;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};
use zbus::{connection::Builder, interface};

pub const BUS_NAME: &str = "org.mpris.MediaPlayer2.rmac_player";
const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";
const TRACK_PREFIX: &str = "/org/rmac/Player/Track/";
const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";

/// What the window publishes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub has_track: bool,
    pub track: usize,
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub url: String,
    pub duration: f64,
    pub position: f64,
    pub volume: f64,
    pub can_go_next: bool,
    pub can_go_previous: bool,
}

/// What MPRIS clients ask the window to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Raise,
    Quit,
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
    SeekTo(f64),
    SetVolume(f64),
    Open(String),
}

type Shared = Arc<Mutex<Snapshot>>;

fn read(shared: &Shared) -> Snapshot {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

struct Root {
    commands: async_channel::Sender<Command>,
}

#[interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {
        let _ = self.commands.try_send(Command::Raise);
    }

    fn quit(&self) {
        let _ = self.commands.try_send(Command::Quit);
    }

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn identity(&self) -> String {
        "Media Player".into()
    }

    #[zbus(property)]
    fn desktop_entry(&self) -> String {
        rmac_apps::identity::PLAYER.into()
    }

    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        vec!["file".into()]
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

struct PlayerInterface {
    shared: Shared,
    commands: async_channel::Sender<Command>,
}

impl PlayerInterface {
    fn send(&self, command: Command) {
        let _ = self.commands.try_send(command);
    }
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl PlayerInterface {
    fn next(&self) {
        self.send(Command::Next);
    }

    fn previous(&self) {
        self.send(Command::Previous);
    }

    fn pause(&self) {
        self.send(Command::Pause);
    }

    fn play_pause(&self) {
        self.send(Command::PlayPause);
    }

    fn stop(&self) {
        self.send(Command::Stop);
    }

    fn play(&self) {
        self.send(Command::Play);
    }

    fn seek(&self, offset: i64) {
        let snapshot = read(&self.shared);
        match model::seek_target(snapshot.position, offset, snapshot.duration) {
            Some(target) => self.send(Command::SeekTo(target)),
            None => self.send(Command::Next),
        }
    }

    fn set_position(&self, track_id: ObjectPath<'_>, position: i64) {
        let snapshot = read(&self.shared);
        let current = format!("{TRACK_PREFIX}{}", snapshot.track);
        let seconds = position as f64 / 1_000_000.0;
        if track_id.as_str() == current && seconds >= 0.0 && seconds <= snapshot.duration {
            self.send(Command::SeekTo(seconds));
        }
    }

    fn open_uri(&self, uri: &str) {
        // The window validates the path; this only keeps oversized requests
        // from being copied into the command queue.
        if uri.len() <= rmac_player::playlist::MAX_URI_BYTES {
            self.send(Command::Open(uri.to_owned()));
        }
    }

    #[zbus(property)]
    fn playback_status(&self) -> String {
        let snapshot = read(&self.shared);
        match (snapshot.has_track, snapshot.playing) {
            (false, _) => "Stopped",
            (true, true) => "Playing",
            (true, false) => "Paused",
        }
        .into()
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        metadata(&read(&self.shared))
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        model::mpris_volume(read(&self.shared).volume)
    }

    #[zbus(property)]
    fn set_volume(&mut self, volume: f64) {
        self.send(Command::SetVolume(model::mpv_volume(volume)));
    }

    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        model::microseconds(read(&self.shared).position)
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        read(&self.shared).can_go_next
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        read(&self.shared).can_go_previous
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        read(&self.shared).has_track
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        read(&self.shared).has_track
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        let snapshot = read(&self.shared);
        snapshot.has_track && snapshot.duration > 0.0
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }

    #[zbus(signal)]
    async fn seeked(emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;
}

fn metadata(snapshot: &Snapshot) -> HashMap<String, OwnedValue> {
    fn insert(map: &mut HashMap<String, OwnedValue>, key: &str, value: Value<'_>) {
        if let Ok(value) = OwnedValue::try_from(value) {
            map.insert(key.to_owned(), value);
        }
    }
    let mut map = HashMap::new();
    let path = if snapshot.has_track {
        format!("{TRACK_PREFIX}{}", snapshot.track)
    } else {
        NO_TRACK.to_owned()
    };
    if let Ok(path) = ObjectPath::try_from(path) {
        insert(&mut map, "mpris:trackid", Value::from(path));
    }
    if snapshot.has_track {
        if snapshot.duration > 0.0 {
            let length = model::microseconds(snapshot.duration);
            insert(&mut map, "mpris:length", Value::from(length));
        }
        for (key, text) in [
            ("xesam:title", &snapshot.title),
            ("xesam:album", &snapshot.album),
            ("xesam:url", &snapshot.url),
        ] {
            if !text.is_empty() {
                insert(&mut map, key, Value::from(text.clone()));
            }
        }
        if !snapshot.artist.is_empty() {
            let artists = vec![snapshot.artist.clone()];
            insert(&mut map, "xesam:artist", Value::from(artists));
        }
    }
    map
}

/// Changes the service should announce.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Notice {
    /// Status, metadata, capabilities or volume changed.
    Changed,
    /// The position jumped (seek); carries seconds.
    Seeked(f64),
}

/// Own the MPRIS name until the window closes; `notices` drives
/// PropertiesChanged and Seeked.
pub async fn serve(
    shared: Arc<Mutex<Snapshot>>,
    commands: async_channel::Sender<Command>,
    notices: async_channel::Receiver<Notice>,
) -> zbus::Result<()> {
    let connection = Builder::session()?
        .name(BUS_NAME)?
        .serve_at(
            OBJECT_PATH,
            Root {
                commands: commands.clone(),
            },
        )?
        .serve_at(OBJECT_PATH, PlayerInterface { shared, commands })?
        .build()
        .await?;
    let player = connection
        .object_server()
        .interface::<_, PlayerInterface>(OBJECT_PATH)
        .await?;
    while let Ok(notice) = notices.recv().await {
        let emitter = player.signal_emitter();
        match notice {
            Notice::Changed => {
                let interface = player.get().await;
                interface.playback_status_changed(emitter).await?;
                interface.metadata_changed(emitter).await?;
                interface.volume_changed(emitter).await?;
                interface.can_go_next_changed(emitter).await?;
                interface.can_go_previous_changed(emitter).await?;
                interface.can_play_changed(emitter).await?;
                interface.can_pause_changed(emitter).await?;
                interface.can_seek_changed(emitter).await?;
            }
            Notice::Seeked(seconds) => {
                PlayerInterface::seeked(emitter, model::microseconds(seconds)).await?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_describes_the_track() {
        let empty = metadata(&Snapshot::default());
        assert_eq!(empty.len(), 1);
        let snapshot = Snapshot {
            has_track: true,
            track: 2,
            title: "Test Tone".into(),
            artist: "rmac".into(),
            duration: 30.0,
            url: "file:///m/tone.m4a".into(),
            ..Snapshot::default()
        };
        let map = metadata(&snapshot);
        assert!(map.contains_key("mpris:trackid"));
        assert!(map.contains_key("xesam:title"));
        assert!(map.contains_key("xesam:artist"));
        assert!(map.contains_key("mpris:length"));
        assert!(!map.contains_key("xesam:album"));
    }
}
