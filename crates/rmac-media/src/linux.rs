use std::collections::HashMap;
use std::path::PathBuf;

use zbus::blocking::{fdo::DBusProxy, Connection, Proxy};
use zbus::zvariant::OwnedValue;

use crate::model::{select_active, Command, Error, ErrorKind, PlaybackStatus, Player, BUS_PREFIX};

const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";
const ROOT_INTERFACE: &str = "org.mpris.MediaPlayer2";
const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";
const LAST_ACTIVE_FILE: &str = "media-last-player";

pub(crate) fn players() -> Result<Vec<Player>, Error> {
    let connection = session()?;
    let names = DBusProxy::new(&connection)
        .and_then(|proxy| proxy.list_names())
        .map_err(|error| Error::new(ErrorKind::Unavailable, error.to_string()))?;
    let mut players = names
        .iter()
        .map(|name| name.as_str())
        .filter(|name| name.starts_with(BUS_PREFIX))
        // A player that stops answering is skipped rather than failing the list.
        .filter_map(|name| read_player(&connection, name).ok())
        .collect::<Vec<_>>();
    players.sort_by(|left, right| left.bus_name.cmp(&right.bus_name));
    Ok(players)
}

pub(crate) fn active_player() -> Result<Option<Player>, Error> {
    let players = players()?;
    let last_active = read_last_active();
    Ok(select_active(&players, last_active.as_deref()).cloned())
}

pub(crate) fn send(command: Command) -> Result<Player, Error> {
    let player = active_player()?
        .filter(|player| command.supported_by(player))
        .ok_or_else(|| Error::new(ErrorKind::NoPlayer, "no media player can take this command"))?;
    let connection = session()?;
    player_proxy(&connection, &player.bus_name, PLAYER_INTERFACE)
        .and_then(|proxy| proxy.call_method(command.method(), &()).map(drop))
        .map_err(|error| Error::new(ErrorKind::Failed, error.to_string()))?;
    write_last_active(&player.bus_name);
    Ok(player)
}

fn session() -> Result<Connection, Error> {
    Connection::session().map_err(|error| Error::new(ErrorKind::Unavailable, error.to_string()))
}

fn player_proxy<'a>(
    connection: &Connection,
    bus_name: &'a str,
    interface: &'a str,
) -> zbus::Result<Proxy<'a>> {
    Proxy::new(connection, bus_name, OBJECT_PATH, interface)
}

fn read_player(connection: &Connection, bus_name: &str) -> zbus::Result<Player> {
    let root = player_proxy(connection, bus_name, ROOT_INTERFACE)?;
    let player = player_proxy(connection, bus_name, PLAYER_INTERFACE)?;
    let flag = |name: &str| player.get_property::<bool>(name).unwrap_or(false);

    let identity = root
        .get_property::<String>("Identity")
        .unwrap_or_else(|_| bus_name.trim_start_matches(BUS_PREFIX).to_owned());
    let status = PlaybackStatus::parse(&player.get_property::<String>("PlaybackStatus")?);
    let metadata = player
        .get_property::<HashMap<String, OwnedValue>>("Metadata")
        .unwrap_or_default();

    Ok(Player {
        bus_name: bus_name.to_owned(),
        identity,
        status,
        title: text(&metadata, "xesam:title"),
        artist: list(&metadata, "xesam:artist"),
        album: text(&metadata, "xesam:album"),
        art_url: text(&metadata, "mpris:artUrl"),
        can_control: flag("CanControl"),
        can_play: flag("CanPlay"),
        can_pause: flag("CanPause"),
        can_go_next: flag("CanGoNext"),
        can_go_previous: flag("CanGoPrevious"),
    })
}

fn text(metadata: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    let value = metadata.get(key)?.try_clone().ok()?;
    String::try_from(value)
        .ok()
        .filter(|value| !value.is_empty())
}

fn list(metadata: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    let value = metadata.get(key)?.try_clone().ok()?;
    let values = Vec::<String>::try_from(value).ok()?;
    let joined = values.join(", ");
    (!joined.is_empty()).then_some(joined)
}

fn last_active_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .map(|runtime| runtime.join("rmac").join(LAST_ACTIVE_FILE))
}

fn read_last_active() -> Option<String> {
    let value = std::fs::read_to_string(last_active_path()?).ok()?;
    let value = value.trim();
    value.starts_with(BUS_PREFIX).then(|| value.to_owned())
}

fn write_last_active(bus_name: &str) {
    let Some(path) = last_active_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, bus_name);
}
