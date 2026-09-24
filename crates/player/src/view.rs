//! The Media Player window: the picture (or the compact audio controller)
//! with QuickTime Player's floating controls.

use std::sync::Arc;
#[cfg(target_os = "linux")]
use std::sync::Mutex;
use std::time::{Duration, Instant};

use gpui::{
    div, img, linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px, rgb, rgba, size,
    svg, ClickEvent, Context, FocusHandle, FontWeight, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Point,
    Render, RenderImage, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    WindowControlArea,
};
use rmac_player::metrics as m;
use rmac_player::model;
use rmac_player::playlist::{self, Kind, Playlist};
use rmac_ui::mac;

use crate::mpv::{Event, Player};
use crate::{
    CloseWindow, NextItem, PlayPause, PreviousItem, SkipBack, SkipForward, ToggleFullScreen,
    ToggleMute, VolumeDown, VolumeUp,
};

/// Rewind and fast-forward jump this far (S: QuickTime scans instead).
const SKIP_SECONDS: f64 = 10.0;
const VOLUME_STEP: f64 = 10.0;
const HIDE_CHECK: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Drag {
    Timeline,
    Volume,
}

#[derive(Clone, Debug, Default)]
struct Playback {
    position: f64,
    duration: f64,
    paused: bool,
    volume: f64,
    muted: bool,
    title: String,
    artist: String,
    album: String,
    video: (f64, f64),
    loaded: bool,
}

pub(crate) struct PlayerView {
    pub(crate) focus: FocusHandle,
    player: Option<Player>,
    error: Option<SharedString>,
    playlist: Playlist,
    playback: Playback,
    frame: Option<Arc<RenderImage>>,
    garbage: Vec<Arc<RenderImage>>,
    last_pointer: Instant,
    show_remaining: bool,
    drag: Option<Drag>,
    /// The window was resized to this video's size already.
    sized_for: Option<(f64, f64)>,
    #[cfg(target_os = "linux")]
    mpris: Arc<Mutex<crate::mpris::Snapshot>>,
    #[cfg(target_os = "linux")]
    notices: Option<async_channel::Sender<crate::mpris::Notice>>,
}

impl PlayerView {
    pub(crate) fn new(playlist: Playlist, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (event_tx, event_rx) = async_channel::unbounded();
        let (frame_tx, frame_rx) = async_channel::bounded(1);
        let (player, error) = match Player::start(event_tx, frame_tx) {
            Ok(player) => (Some(player), None),
            Err(error) => (None, Some(SharedString::from(error))),
        };
        let mut view = Self {
            focus: cx.focus_handle(),
            player,
            error,
            playlist,
            playback: Playback {
                volume: 100.0,
                ..Playback::default()
            },
            frame: None,
            garbage: Vec::new(),
            last_pointer: Instant::now(),
            show_remaining: false,
            drag: None,
            sized_for: None,
            #[cfg(target_os = "linux")]
            mpris: Arc::new(Mutex::new(crate::mpris::Snapshot::default())),
            #[cfg(target_os = "linux")]
            notices: None,
        };
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = event_rx.recv().await {
                if this
                    .update_in(cx, |view, window, cx| view.apply(event, window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            while frame_rx.recv().await.is_ok() {
                if this.update(cx, |view, cx| view.take_frame(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(HIDE_CHECK).await;
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }
        })
        .detach();
        view.start_mpris(window, cx);
        view.load_current();
        view
    }

    #[cfg(target_os = "linux")]
    fn start_mpris(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (command_tx, command_rx) = async_channel::bounded(16);
        let (notice_tx, notice_rx) = async_channel::unbounded();
        self.notices = Some(notice_tx);
        let shared = self.mpris.clone();
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = crate::mpris::serve(shared, command_tx, notice_rx).await {
                    eprintln!("rmac-player: MPRIS is unavailable: {error}");
                }
            })
            .detach();
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(command) = command_rx.recv().await {
                if this
                    .update_in(cx, |view, window, cx| {
                        view.mpris_command(command, window, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    #[cfg(not(target_os = "linux"))]
    fn start_mpris(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    #[cfg(target_os = "linux")]
    fn mpris_command(
        &mut self,
        command: crate::mpris::Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::mpris::Command;
        match command {
            Command::Raise => window.activate_window(),
            Command::Quit => window.remove_window(),
            Command::Play => self.set_paused(false),
            Command::Pause | Command::Stop => self.set_paused(true),
            Command::PlayPause => self.toggle_play(),
            Command::Next => self.next(cx),
            Command::Previous => self.previous(cx),
            Command::SeekTo(seconds) => self.seek_to(seconds),
            Command::SetVolume(volume) => {
                if let Some(player) = &self.player {
                    player.set_volume(volume);
                }
            }
            Command::Open(uri) => {
                if let Some(path) = playlist::path_from_open_uri(&uri) {
                    if self.playlist.append([path]).is_some() {
                        self.load_current();
                    }
                }
            }
        }
        cx.notify();
    }

    /// Publish the state MPRIS clients read and announce the change.
    fn publish(&self, seeked: Option<f64>) {
        #[cfg(target_os = "linux")]
        {
            let snapshot = crate::mpris::Snapshot {
                has_track: self.playback.loaded,
                track: self.playlist.index() + 1,
                playing: self.playback.loaded && !self.playback.paused,
                title: self.title(),
                artist: self.playback.artist.clone(),
                album: self.playback.album.clone(),
                url: self
                    .playlist
                    .current()
                    .map(|path| format!("file://{}", path.to_string_lossy()))
                    .unwrap_or_default(),
                duration: self.playback.duration,
                position: self.playback.position,
                volume: self.playback.volume,
                can_go_next: self.playlist.has_next(),
                can_go_previous: self.playback.loaded,
            };
            let changed = {
                let mut shared = self
                    .mpris
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let changed = crate::mpris::Snapshot {
                    position: shared.position,
                    ..snapshot.clone()
                } != *shared;
                *shared = snapshot;
                changed
            };
            if let Some(notices) = &self.notices {
                if changed {
                    let _ = notices.try_send(crate::mpris::Notice::Changed);
                }
                if let Some(seconds) = seeked {
                    let _ = notices.try_send(crate::mpris::Notice::Seeked(seconds));
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = seeked;
    }

    fn apply(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        let playback = &mut self.playback;
        match event {
            Event::TimePosition(position) => playback.position = position,
            Event::Duration(duration) => playback.duration = duration,
            Event::Paused(paused) => playback.paused = paused,
            Event::Volume(volume) => playback.volume = volume,
            Event::Muted(muted) => playback.muted = muted,
            Event::Title(title) => playback.title = title,
            Event::Artist(artist) => playback.artist = artist,
            Event::Album(album) => playback.album = album,
            Event::FileLoaded => playback.loaded = true,
            Event::VideoSize(width, height) => {
                playback.video = (width, height);
                self.fit_window(window);
            }
            Event::Ended(true) => {
                if self.playlist.has_next() {
                    self.next(cx);
                } else if let Some(player) = &self.player {
                    // Stay on the last frame, paused, like QuickTime.
                    player.set_paused(true);
                }
            }
            Event::Ended(false) => {
                self.error = Some("The file could not be played.".into());
            }
            Event::Shutdown => self.player = None,
        }
        self.publish(None);
        cx.notify();
    }

    /// Size a video window to the picture the first time it is known.
    fn fit_window(&mut self, window: &mut Window) {
        let video = self.playback.video;
        if self.kind() != Some(Kind::Video)
            || video.0 <= 0.0
            || video.1 <= 0.0
            || self.sized_for == Some(video)
            || window.is_fullscreen()
        {
            return;
        }
        self.sized_for = Some(video);
        let (width, height) =
            model::video_window_size(video, m::DEFAULT_VIDEO_WINDOW, m::MIN_VIDEO_WINDOW);
        window.resize(size(px(width), px(height)));
    }

    fn take_frame(&mut self, cx: &mut Context<Self>) {
        let Some(frame) = self.player.as_ref().and_then(Player::take_frame) else {
            return;
        };
        let Some(buffer) = image::RgbaImage::from_raw(frame.width, frame.height, frame.pixels)
        else {
            return;
        };
        let image = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
        if let Some(old) = self.frame.replace(image) {
            self.garbage.push(old);
        }
        cx.notify();
    }

    fn kind(&self) -> Option<Kind> {
        self.playlist.current().and_then(playlist::kind)
    }

    fn title(&self) -> String {
        let fallback = self
            .playlist
            .current()
            .map(playlist::display_name)
            .unwrap_or_default();
        let title = self.playback.title.trim();
        // mpv reports the file name when a file has no title tag.
        if title.is_empty()
            || self
                .playlist
                .current()
                .and_then(|path| path.file_name())
                .is_some_and(|name| name.to_string_lossy() == title)
        {
            fallback
        } else {
            title.to_owned()
        }
    }

    fn load_current(&mut self) {
        let Some(path) = self.playlist.current().map(|path| path.to_path_buf()) else {
            return;
        };
        self.playback = Playback {
            volume: self.playback.volume,
            muted: self.playback.muted,
            ..Playback::default()
        };
        self.error = None;
        if let Some(player) = &self.player {
            player.load(&path);
        }
        self.publish(None);
    }

    fn set_paused(&mut self, paused: bool) {
        if let Some(player) = &self.player {
            let at_end = self.playback.duration > 0.0
                && self.playback.position >= self.playback.duration - 0.25;
            if !paused && at_end {
                player.seek_to(0.0);
            }
            player.set_paused(paused);
        }
    }

    fn toggle_play(&mut self) {
        self.set_paused(!self.playback.paused);
    }

    fn seek_to(&mut self, seconds: f64) {
        if let Some(player) = &self.player {
            player.seek_to(seconds);
            self.playback.position = seconds;
            self.publish(Some(seconds));
        }
    }

    fn skip(&mut self, seconds: f64) {
        let duration = self.playback.duration.max(0.0);
        let target = (self.playback.position + seconds).clamp(0.0, duration);
        self.seek_to(target);
    }

    fn next(&mut self, cx: &mut Context<Self>) {
        if self.playlist.advance().is_some() {
            self.sized_for = None;
            self.load_current();
            cx.notify();
        }
    }

    fn previous(&mut self, cx: &mut Context<Self>) {
        if self.playlist.previous(self.playback.position).is_some() {
            self.sized_for = None;
            self.load_current();
        } else {
            self.seek_to(0.0);
        }
        cx.notify();
    }

    fn change_volume(&mut self, delta: f64) {
        if let Some(player) = &self.player {
            player.set_volume(self.playback.volume + delta);
            if self.playback.muted {
                player.set_muted(false);
            }
        }
    }

    // ------------------------------------------------------------ pointer

    fn timeline_geometry(&self, width: f32) -> (f32, f32) {
        if self.kind() == Some(Kind::Audio) {
            (m::AUDIO_TIMELINE.0, m::AUDIO_TIMELINE.2)
        } else {
            let plate = (width - m::HUD_WIDTH) / 2.0;
            (plate + m::HUD_TIMELINE.0, m::HUD_TIMELINE.2)
        }
    }

    fn volume_geometry(&self, width: f32) -> (f32, f32) {
        if self.kind() == Some(Kind::Audio) {
            (m::AUDIO_VOLUME.0, m::AUDIO_VOLUME.2)
        } else {
            let plate = (width - m::HUD_WIDTH) / 2.0;
            (plate + m::HUD_VOLUME.0, m::HUD_VOLUME.2)
        }
    }

    fn drag_to(&mut self, drag: Drag, position: Point<Pixels>, window: &Window) {
        let width = f32::from(window.viewport_size().width);
        let x = f32::from(position.x);
        match drag {
            Drag::Timeline => {
                let (left, track) = self.timeline_geometry(width);
                let seconds = model::scrub_position(x - left, track, self.playback.duration);
                self.seek_to(seconds);
            }
            Drag::Volume => {
                let (left, track) = self.volume_geometry(width);
                let volume = f64::from(((x - left) / track).clamp(0.0, 1.0)) * 100.0;
                if let Some(player) = &self.player {
                    player.set_volume(volume);
                    player.set_muted(false);
                }
                self.playback.volume = volume;
            }
        }
    }

    // ------------------------------------------------------------ pieces

    fn glyph(
        &self,
        id: &'static str,
        path: &'static str,
        glyph: f32,
        center: (f32, f32),
    ) -> gpui::Stateful<gpui::Div> {
        let hit = glyph + 12.0;
        div()
            .id(id)
            .absolute()
            .left(px(center.0 - hit / 2.0))
            .top(px(center.1 - hit / 2.0))
            .size(px(hit))
            .flex()
            .items_center()
            .justify_center()
            .active(|style| style.opacity(0.6))
            .child(svg().path(path).size(px(glyph)).text_color(mac::white()))
    }

    #[allow(clippy::too_many_arguments)]
    fn track(
        &self,
        id: &'static str,
        left: f32,
        center_y: f32,
        width: f32,
        fraction: f32,
        thumb: f32,
        drag: Drag,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .absolute()
            .left(px(left))
            .top(px(center_y - thumb / 2.0))
            .w(px(width))
            .h(px(thumb))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.drag = Some(drag);
                    this.drag_to(drag, event.position, window);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px((thumb - m::TRACK_HEIGHT) / 2.0))
                    .w_full()
                    .h(px(m::TRACK_HEIGHT))
                    .rounded(px(m::TRACK_HEIGHT / 2.0))
                    .bg(rgba(m::TRACK_FILL))
                    .child(
                        div()
                            .h_full()
                            .w(px(width * fraction))
                            .rounded(px(m::TRACK_HEIGHT / 2.0))
                            .bg(mac::white()),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left(px(width * fraction - thumb / 2.0))
                    .top_0()
                    .size(px(thumb))
                    .rounded_full()
                    .bg(mac::white())
                    .shadow_sm(),
            )
    }

    fn time_label(
        &self,
        id: &'static str,
        text: String,
        left: f32,
        top: f32,
        align_right: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(m::TIME_WIDTH))
            .h(px(17.0))
            .flex()
            .items_center()
            .when(align_right, |label| label.justify_end())
            .text_size(px(m::TIME_SIZE))
            .text_color(mac::white())
            .font_features(mac::tabular_font_features())
            .child(text)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.show_remaining = !this.show_remaining;
                cx.notify();
            }))
    }

    fn duration_text(&self) -> String {
        if self.show_remaining {
            model::remaining_text(self.playback.position, self.playback.duration)
        } else {
            model::clock_text(self.playback.duration)
        }
    }

    fn transport(
        &self,
        rewind: (f32, f32),
        play: (f32, f32),
        forward: (f32, f32),
        play_glyph: f32,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let playing = self.playback.loaded && !self.playback.paused;
        vec![
            self.glyph(
                "player-rewind",
                "icons/player/rewind.svg",
                m::TRANSPORT_GLYPH,
                rewind,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.skip(-SKIP_SECONDS);
                cx.notify();
            }))
            .into_any_element(),
            self.glyph(
                "player-play",
                if playing {
                    "icons/player/pause.svg"
                } else {
                    "icons/player/play.svg"
                },
                play_glyph,
                play,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.toggle_play();
                cx.notify();
            }))
            .into_any_element(),
            self.glyph(
                "player-forward",
                "icons/player/forward.svg",
                m::TRANSPORT_GLYPH,
                forward,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.skip(SKIP_SECONDS);
                cx.notify();
            }))
            .into_any_element(),
        ]
    }

    fn speaker(&self, left: f32, top: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = self.playback.muted || self.playback.volume <= 0.0;
        div()
            .id("player-mute")
            .absolute()
            .left(px(left))
            .top(px(top - 3.0))
            .size(px(m::SPEAKER_GLYPH))
            .active(|style| style.opacity(0.6))
            .child(
                svg()
                    .path(if muted {
                        "icons/player/mute.svg"
                    } else {
                        "icons/player/volume.svg"
                    })
                    .size(px(m::SPEAKER_GLYPH))
                    .text_color(mac::white()),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                if let Some(player) = &this.player {
                    player.set_muted(!this.playback.muted);
                }
                cx.notify();
            }))
    }

    fn render_audio(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let progress = model::progress(self.playback.position, self.playback.duration);
        let volume = if self.playback.muted {
            0.0
        } else {
            (self.playback.volume / 100.0).clamp(0.0, 1.0) as f32
        };
        div()
            .absolute()
            .size_full()
            .bg(rgba(m::AUDIO_FILL))
            .child(self.time_label(
                "player-elapsed",
                model::clock_text(self.playback.position),
                m::AUDIO_ELAPSED.0,
                m::AUDIO_ELAPSED.1,
                false,
                cx,
            ))
            .child(self.track(
                "player-timeline",
                m::AUDIO_TIMELINE.0,
                m::AUDIO_TIMELINE.1,
                m::AUDIO_TIMELINE.2,
                progress,
                m::THUMB_DIAMETER,
                Drag::Timeline,
                cx,
            ))
            .child(self.time_label(
                "player-duration",
                self.duration_text(),
                m::AUDIO_DURATION.0,
                m::AUDIO_DURATION.1,
                true,
                cx,
            ))
            .children(self.transport(
                m::AUDIO_REWIND_CENTER,
                m::AUDIO_PLAY_CENTER,
                m::AUDIO_FORWARD_CENTER,
                30.0,
                cx,
            ))
            .child(self.speaker(m::AUDIO_MUTE.0, m::AUDIO_MUTE.1, cx))
            .child(self.track(
                "player-volume",
                m::AUDIO_VOLUME.0,
                m::AUDIO_VOLUME.1,
                m::AUDIO_VOLUME.2,
                volume,
                m::VOLUME_THUMB_DIAMETER,
                Drag::Volume,
                cx,
            ))
    }

    fn render_hud(
        &self,
        width: f32,
        height: f32,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let left = ((width - m::HUD_WIDTH) / 2.0).round();
        let top = height - m::HUD_BOTTOM - m::HUD_HEIGHT;
        let progress = model::progress(self.playback.position, self.playback.duration);
        let volume = if self.playback.muted {
            0.0
        } else {
            (self.playback.volume / 100.0).clamp(0.0, 1.0) as f32
        };
        let fullscreen = window.is_fullscreen();
        div()
            .id("player-hud")
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(m::HUD_WIDTH))
            .h(px(m::HUD_HEIGHT))
            .rounded(px(m::HUD_RADIUS))
            .bg(rgba(m::HUD_FILL))
            .border_1()
            .border_color(rgba(m::HUD_RIM))
            .shadow_lg()
            // Clicks on the plate never reach the picture underneath.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(self.speaker(m::HUD_MUTE.0, m::HUD_MUTE.1, cx))
            .child(self.track(
                "player-volume",
                m::HUD_VOLUME.0,
                m::HUD_VOLUME.1,
                m::HUD_VOLUME.2,
                volume,
                12.0,
                Drag::Volume,
                cx,
            ))
            .children(self.transport(
                m::HUD_REWIND_CENTER,
                m::HUD_PLAY_CENTER,
                m::HUD_FORWARD_CENTER,
                m::PLAY_GLYPH,
                cx,
            ))
            .child(
                self.glyph(
                    "player-fullscreen",
                    if fullscreen {
                        "icons/player/exit-fullscreen.svg"
                    } else {
                        "icons/player/fullscreen.svg"
                    },
                    18.0,
                    m::HUD_FULLSCREEN_CENTER,
                )
                .on_click(|_, window, _| window.toggle_fullscreen()),
            )
            .child(self.time_label(
                "player-elapsed",
                model::clock_text(self.playback.position),
                m::HUD_ELAPSED.0,
                m::HUD_ELAPSED.1,
                false,
                cx,
            ))
            .child(self.track(
                "player-timeline",
                m::HUD_TIMELINE.0,
                m::HUD_TIMELINE.1,
                m::HUD_TIMELINE.2,
                progress,
                m::THUMB_DIAMETER,
                Drag::Timeline,
                cx,
            ))
            .child(self.time_label(
                "player-duration",
                self.duration_text(),
                m::HUD_WIDTH - m::HUD_DURATION_RIGHT - m::TIME_WIDTH,
                m::HUD_ELAPSED.1,
                true,
                cx,
            ))
    }

    fn render_title_bar(&self, audio: bool, window: &Window) -> impl IntoElement {
        let (light_x, light_y) = m::TRAFFIC_LIGHT_CENTER;
        div()
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .h(px(m::TITLE_BAR_HEIGHT))
            .when(!audio, |bar| {
                bar.bg(linear_gradient(
                    180.0,
                    linear_color_stop(rgba(0x0000_0073), 0.0),
                    linear_color_stop(rgba(0x0000_0000), 1.0),
                ))
            })
            .child(
                div()
                    .id("player-drag")
                    .absolute()
                    .size_full()
                    .window_control_area(WindowControlArea::Drag),
            )
            .child(
                div()
                    .absolute()
                    .left(px(light_x - mac::traffic_light_hit_width() / 2.0))
                    .top(px(light_y - mac::traffic_light_hit_height() / 2.0))
                    .child(if audio {
                        rmac_ui::traffic_lights_fixed_size(window.is_window_active())
                    } else {
                        rmac_ui::traffic_lights_active(window.is_window_active())
                    }),
            )
            .child(
                div()
                    .absolute()
                    .left(px(m::TITLE_ICON_LEFT))
                    .top(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(3.0))
                    .child(
                        svg()
                            .path("icons/player/audio-file.svg")
                            .size(px(16.0))
                            .text_color(mac::white()),
                    )
                    .child(
                        div()
                            .text_size(px(m::TITLE_SIZE))
                            .font_weight(FontWeight::BOLD)
                            .text_color(mac::white())
                            .whitespace_nowrap()
                            .child(self.title()),
                    ),
            )
    }
}

impl Render for PlayerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for image in self.garbage.drain(..) {
            cx.drop_image(image, Some(window));
        }
        let viewport = window.viewport_size();
        let (width, height) = (f32::from(viewport.width), f32::from(viewport.height));
        let audio = self.kind() == Some(Kind::Audio);
        let video = self.playback.video;
        // The picture: contained in the window, rendered at that size.
        let picture = (!audio && video.0 > 0.0 && video.1 > 0.0).then(|| {
            let fit = (f64::from(width) / video.0).min(f64::from(height) / video.1);
            ((video.0 * fit) as f32, (video.1 * fit) as f32)
        });
        if let Some(player) = &self.player {
            player.set_render_size(
                picture
                    .and_then(|display| model::render_size(display, window.scale_factor(), video)),
            );
        }
        let playing = self.playback.loaded && !self.playback.paused;
        let idle = self.last_pointer.elapsed().as_millis() as u64;
        let controls = model::controls_visible(playing, !audio, idle, self.drag.is_some());
        let frame = self.frame.clone();
        let error = self.error.clone();
        div()
            .id("player")
            .track_focus(&self.focus)
            .key_context("Player")
            .on_action(cx.listener(|this, _: &PlayPause, _, cx| {
                this.toggle_play();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SkipBack, _, cx| {
                this.skip(-SKIP_SECONDS);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SkipForward, _, cx| {
                this.skip(SKIP_SECONDS);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &VolumeUp, _, _| this.change_volume(VOLUME_STEP)))
            .on_action(cx.listener(|this, _: &VolumeDown, _, _| this.change_volume(-VOLUME_STEP)))
            .on_action(cx.listener(|this, _: &ToggleMute, _, _| {
                if let Some(player) = &this.player {
                    player.set_muted(!this.playback.muted);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleFullScreen, window, _| {
                if this.kind() == Some(Kind::Video) {
                    window.toggle_fullscreen();
                }
            }))
            .on_action(cx.listener(|this, _: &NextItem, _, cx| this.next(cx)))
            .on_action(cx.listener(|this, _: &PreviousItem, _, cx| this.previous(cx)))
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .on_action(
                cx.listener(|_, _: &rmac_ui::RequestClose, window, _| window.remove_window()),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                this.last_pointer = Instant::now();
                if let (Some(drag), Some(MouseButton::Left)) = (this.drag, event.pressed_button) {
                    this.drag_to(drag, event.position, window);
                }
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    if this.drag.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(if audio { 0x262424 } else { 0x000000 }))
            .when_some(
                picture.zip(frame),
                |root, ((picture_width, picture_height), frame)| {
                    root.child(
                        div()
                            .id("player-picture")
                            .absolute()
                            .left(px((width - picture_width) / 2.0))
                            .top(px((height - picture_height) / 2.0))
                            .w(px(picture_width))
                            .h(px(picture_height))
                            .child(img(frame).size_full())
                            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                                if event.click_count() >= 2 {
                                    window.toggle_fullscreen();
                                } else {
                                    this.toggle_play();
                                }
                                cx.notify();
                            })),
                    )
                },
            )
            .when(audio, |root| root.child(self.render_audio(cx)))
            .when(!audio && controls, |root| {
                root.child(self.render_hud(width, height, window, cx))
            })
            .when(audio || controls, |root| {
                root.child(self.render_title_bar(audio, window))
            })
            .children(error.map(|message| {
                div()
                    .absolute()
                    .left_0()
                    .w_full()
                    .top(px(if audio { 12.0 } else { height / 2.0 - 10.0 }))
                    .flex()
                    .justify_center()
                    .text_size(px(13.0))
                    .text_color(mac::white())
                    .when(audio, |label| label.top(px(34.0)).text_size(px(11.0)))
                    .child(message)
            }))
    }
}
