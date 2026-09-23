use super::*;

fn player(name: &str, status: PlaybackStatus, title: Option<&str>) -> Player {
    Player {
        bus_name: format!("{BUS_PREFIX}{name}"),
        identity: name.into(),
        status,
        title: title.map(Into::into),
        artist: None,
        album: None,
        art_url: None,
        can_control: true,
        can_play: true,
        can_pause: true,
        can_go_next: true,
        can_go_previous: true,
    }
}

#[test]
fn playing_player_wins_over_last_controlled_paused_player() {
    let players = [
        player("firefox", PlaybackStatus::Paused, Some("Video")),
        player("spotify", PlaybackStatus::Playing, Some("Song")),
    ];
    let active = select_active(&players, Some("org.mpris.MediaPlayer2.firefox")).unwrap();
    assert_eq!(active.identity, "spotify");
}

#[test]
fn last_controlled_breaks_ties_between_playing_players() {
    let players = [
        player("firefox", PlaybackStatus::Playing, Some("Video")),
        player("spotify", PlaybackStatus::Playing, Some("Song")),
    ];
    let active = select_active(&players, Some("org.mpris.MediaPlayer2.spotify")).unwrap();
    assert_eq!(active.identity, "spotify");
}

#[test]
fn paused_player_with_a_track_is_resumable() {
    let players = [
        player("idle", PlaybackStatus::Stopped, None),
        player("spotify", PlaybackStatus::Paused, Some("Song")),
    ];
    assert_eq!(select_active(&players, None).unwrap().identity, "spotify");
}

#[test]
fn stopped_players_without_tracks_and_uncontrollable_players_are_ignored() {
    let mut locked = player("kiosk", PlaybackStatus::Playing, Some("Ad"));
    locked.can_control = false;
    let players = [player("idle", PlaybackStatus::Stopped, None), locked];
    assert_eq!(select_active(&players, None), None);
}

#[test]
fn commands_respect_player_capabilities() {
    let mut radio = player("radio", PlaybackStatus::Playing, Some("Live"));
    radio.can_go_next = false;
    assert!(!Command::Next.supported_by(&radio));
    assert!(Command::PlayPause.supported_by(&radio));
    assert_eq!(Command::parse("play-pause"), Some(Command::PlayPause));
    assert_eq!(Command::parse("rewind"), None);
    assert_eq!(PlaybackStatus::parse("Buffering"), PlaybackStatus::Stopped);
}
