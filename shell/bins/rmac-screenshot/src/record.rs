//! ⇧⌘5 screen recording (SHOT-01).
//!
//! niri serves screen casts as PipeWire streams through the
//! `org.gnome.Mutter.ScreenCast` D-Bus API that the desktop portal also uses.
//! This asks it for a stream of the focused output directly (no share
//! dialog: the shell is the session's own component), then runs a GStreamer
//! pipeline that reads the stream, crops it to a selection, and writes H.264
//! in a QuickTime `.mov` like the Mac, or VP8 in `.webm` when the machine
//! has no H.264 encoder (gstreamer1.0-plugins-ugly).
//!
//! The recording is stopped by sending SIGINT to `gst-launch-1.0 -e`, which
//! ends the stream cleanly and finalises the file. A small file under
//! `$XDG_RUNTIME_DIR/rmac` tells the menu bar a recording is running, so it
//! can show the stop button.

use std::path::{Path, PathBuf};

/// The recording frame rate: smooth, and cheap enough for the target PCs.
pub const FRAME_RATE: u32 = 30;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Encoder {
    /// `x264enc` into a QuickTime movie, as the Mac records.
    H264,
    /// `vp8enc` into WebM: always in GStreamer's good plugins.
    Vp8,
}

impl Encoder {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::H264 => "mov",
            Self::Vp8 => "webm",
        }
    }
}

/// Pixels to cut from each side of the full-output stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Crop {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

/// The crop for `selection` (output-local logical points: x, y, width,
/// height) on an output whose stream is `stream` pixels at `scale` pixels
/// per point. Encoders want even sizes and offsets, so every edge rounds
/// down to even and the result is never empty.
pub fn crop_for(selection: (f32, f32, f32, f32), scale: f32, stream: (u32, u32)) -> Crop {
    let even = |value: u32| value & !1;
    let pixels = |points: f32| (points * scale).round().max(0.0) as u32;
    let axis = |start: f32, length: f32, total: u32| {
        let total = even(total).max(2);
        let begin = even(pixels(start)).min(total - 2);
        let size = even(pixels(length).max(2)).min(total - begin).max(2);
        (begin, total - begin - size)
    };
    let (left, right) = axis(selection.0, selection.2, stream.0);
    let (top, bottom) = axis(selection.1, selection.3, stream.1);
    Crop {
        left,
        top,
        right,
        bottom,
    }
}

/// The `gst-launch-1.0` arguments recording PipeWire node `node` to `out`.
pub fn pipeline_arguments(
    node: u32,
    crop: Option<Crop>,
    encoder: Encoder,
    out: &Path,
) -> Vec<String> {
    let mut stages: Vec<String> = vec![
        format!("pipewiresrc path={node} do-timestamp=true"),
        "queue max-size-buffers=4 leaky=downstream".to_owned(),
        "videoconvert".to_owned(),
    ];
    if let Some(crop) = crop {
        stages.push(format!(
            "videocrop left={} top={} right={} bottom={}",
            crop.left, crop.top, crop.right, crop.bottom
        ));
    }
    stages.push("videorate".to_owned());
    stages.push(format!(
        "video/x-raw,format=I420,framerate={FRAME_RATE}/1"
    ));
    match encoder {
        Encoder::H264 => {
            stages.push(format!(
                "x264enc speed-preset=ultrafast tune=zerolatency bframes=0 pass=quant quantizer=24 key-int-max={}",
                FRAME_RATE * 2
            ));
            stages.push("h264parse".to_owned());
            stages.push("qtmux".to_owned());
        }
        Encoder::Vp8 => {
            stages.push(format!(
                "vp8enc deadline=1 cpu-used=8 threads=2 end-usage=vbr target-bitrate=6000000 keyframe-max-dist={}",
                FRAME_RATE * 2
            ));
            stages.push("webmmux".to_owned());
        }
    }
    stages.push(format!("filesink location={}", out.display()));
    let mut arguments = vec!["-q".to_owned(), "-e".to_owned()];
    for (index, stage) in stages.iter().enumerate() {
        if index > 0 {
            arguments.push("!".to_owned());
        }
        arguments.extend(stage.split(' ').map(str::to_owned));
    }
    arguments
}

/// "Screen Recording 2026-10-20 at 11.22.33 AM.mov", named as macOS names it.
pub fn file_name(time: &chrono::NaiveDateTime, encoder: Encoder) -> String {
    format!(
        "Screen Recording {} at {}\u{202F}{}.{}",
        time.format("%Y-%m-%d"),
        time.format("%-I.%M.%S"),
        time.format("%p"),
        encoder.extension(),
    )
}

/// What the menu bar reads while a recording runs: the start time in Unix
/// milliseconds and the file being written.
pub fn state_text(started_unix_ms: u64, path: &Path) -> String {
    format!("{started_unix_ms}\n{}\n", path.display())
}

pub fn parse_state(text: &str) -> Option<(u64, PathBuf)> {
    let mut lines = text.lines();
    let started = lines.next()?.trim().parse().ok()?;
    let path = lines.next()?.trim();
    (!path.is_empty()).then(|| (started, PathBuf::from(path)))
}

#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::HashMap;
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedObjectPath, Value};

    use super::{parse_state, pipeline_arguments, state_text, Crop, Encoder};

    const BUS: &str = "org.gnome.Mutter.ScreenCast";
    const ROOT: &str = "/org/gnome/Mutter/ScreenCast";
    /// niri has to bring the stream up before it announces the node.
    const STREAM_TIMEOUT: Duration = Duration::from_secs(5);
    /// How long a stopping recorder has to finish writing the file.
    const FINISH_TIMEOUT: Duration = Duration::from_secs(10);
    /// Mutter's cursor modes: 0 hidden, 1 drawn into the picture.
    const CURSOR_HIDDEN: u32 = 0;
    const CURSOR_EMBEDDED: u32 = 1;

    /// The H.264 encoder when GStreamer has one, else VP8.
    pub fn detect_encoder() -> Result<Encoder, String> {
        let has = |element: &str| {
            Command::new("gst-inspect-1.0")
                .arg(element)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        };
        if has("x264enc") && has("qtmux") && has("h264parse") {
            Ok(Encoder::H264)
        } else if has("vp8enc") && has("webmmux") {
            Ok(Encoder::Vp8)
        } else {
            Err("screen recording needs GStreamer (gstreamer1.0-tools and gstreamer1.0-plugins-good)"
                .to_owned())
        }
    }

    /// A ScreenCast session niri keeps for one recording.
    pub struct Session {
        connection: Connection,
        session: OwnedObjectPath,
        pub node: u32,
    }

    impl Session {
        /// Ask niri for a stream of `connector` and wait for its PipeWire node.
        pub fn open(connector: &str, show_pointer: bool) -> Result<Self, String> {
            let error = |what: &str, cause: zbus::Error| format!("screen cast {what}: {cause}");
            let connection = Connection::session().map_err(|e| error("bus", e))?;
            let screen_cast = Proxy::new(&connection, BUS, ROOT, "org.gnome.Mutter.ScreenCast")
                .map_err(|e| error("proxy", e))?;
            let session: OwnedObjectPath = screen_cast
                .call("CreateSession", &(HashMap::<&str, Value>::new(),))
                .map_err(|e| error("session", e))?;
            let session_proxy = Proxy::new(
                &connection,
                BUS,
                session.clone(),
                "org.gnome.Mutter.ScreenCast.Session",
            )
            .map_err(|e| error("session proxy", e))?;
            let mut options = HashMap::<&str, Value>::new();
            options.insert(
                "cursor-mode",
                Value::U32(if show_pointer {
                    CURSOR_EMBEDDED
                } else {
                    CURSOR_HIDDEN
                }),
            );
            let stream: OwnedObjectPath = session_proxy
                .call("RecordMonitor", &(connector, options))
                .map_err(|e| error("monitor", e))?;
            let stream_proxy = Proxy::new(
                &connection,
                BUS,
                stream.clone(),
                "org.gnome.Mutter.ScreenCast.Stream",
            )
            .map_err(|e| error("stream proxy", e))?;
            // The node arrives as a signal after Start, so listen first.
            let signals = stream_proxy
                .receive_signal("PipeWireStreamAdded")
                .map_err(|e| error("signal", e))?;
            let (sender, receiver) = mpsc::channel();
            std::thread::Builder::new()
                .name("rmac-screencast-node".into())
                .spawn(move || {
                    for message in signals {
                        if let Ok(node) = message.body().deserialize::<u32>() {
                            let _ = sender.send(node);
                            return;
                        }
                    }
                })
                .map_err(|e| format!("screen cast thread: {e}"))?;
            session_proxy
                .call::<_, _, ()>("Start", &())
                .map_err(|e| error("start", e))?;
            let node = receiver
                .recv_timeout(STREAM_TIMEOUT)
                .map_err(|_| "niri did not start the screen stream".to_owned())?;
            Ok(Self {
                connection,
                session,
                node,
            })
        }

        pub fn close(self) {
            if let Ok(proxy) = Proxy::new(
                &self.connection,
                BUS,
                self.session.clone(),
                "org.gnome.Mutter.ScreenCast.Session",
            ) {
                let _ = proxy.call::<_, _, ()>("Stop", &());
            }
        }
    }

    /// A running recording: niri's stream, and the pipeline reading it.
    pub struct Recording {
        pub session: Session,
        pub pid: u32,
        pub path: PathBuf,
    }

    /// Start recording. Returns the recording and a receiver that yields when
    /// the pipeline ends, by a stop or by itself.
    pub fn start(
        connector: &str,
        show_pointer: bool,
        crop: Option<Crop>,
        encoder: Encoder,
        path: PathBuf,
    ) -> Result<(Recording, mpsc::Receiver<io::Result<std::process::ExitStatus>>), String> {
        let session = Session::open(connector, show_pointer)?;
        let arguments = pipeline_arguments(session.node, crop, encoder, &path);
        let spawned = Command::new("gst-launch-1.0")
            .args(&arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn();
        let mut child: Child = match spawned {
            Ok(child) => child,
            Err(error) => {
                session.close();
                return Err(format!("could not start the recorder: {error}"));
            }
        };
        let pid = child.id();
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("rmac-screen-recorder".into())
            .spawn(move || {
                let _ = sender.send(child.wait());
            })
            .map_err(|error| format!("recorder thread: {error}"))?;
        Ok((Recording { session, pid, path }, receiver))
    }

    /// Ask the recorder to finish: SIGINT makes `gst-launch-1.0 -e` send
    /// end-of-stream and close the file. After [`FINISH_TIMEOUT`] the caller
    /// should [`kill`].
    pub fn request_stop(pid: u32) {
        // SAFETY: SIGINT to the recorder this process started.
        let _ = unsafe { libc::kill(pid as i32, libc::SIGINT) };
    }

    pub fn kill(pid: u32) {
        // SAFETY: SIGKILL to the recorder this process started.
        let _ = unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    }

    pub const fn finish_timeout() -> Duration {
        FINISH_TIMEOUT
    }

    fn state_path() -> Option<PathBuf> {
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .map(|runtime| runtime.join("rmac/screen-recording"))
    }

    pub fn write_state(started_unix_ms: u64, path: &Path) {
        let Some(file) = state_path() else {
            return;
        };
        if let Some(parent) = file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let temporary = file.with_extension("tmp");
        if fs::write(&temporary, state_text(started_unix_ms, path)).is_ok() {
            let _ = fs::rename(&temporary, &file);
        }
    }

    pub fn clear_state() {
        if let Some(file) = state_path() {
            let _ = fs::remove_file(file);
        }
    }

    /// The running recording the state file describes, if its recorder
    /// still lives (a crashed session leaves a stale file).
    pub fn read_state() -> Option<(u64, PathBuf)> {
        let text = fs::read_to_string(state_path()?).ok()?;
        parse_state(&text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops_round_to_even_pixels_and_stay_inside_the_stream() {
        // A 1536 x 864 point output at 1.25 is 1920 x 1080 pixels.
        let crop = crop_for((100.0, 50.0, 400.0, 300.0), 1.25, (1920, 1080));
        assert_eq!(crop.left, 124); // 125 rounded down to even
        assert_eq!(crop.top, 62); // 62.5 -> 63 -> 62
        let width = 1920 - crop.left - crop.right;
        let height = 1080 - crop.top - crop.bottom;
        assert_eq!((width, height), (500, 374)); // 375 -> 374: even
        assert!(width % 2 == 0 && height % 2 == 0);
        // A selection past the edge is clipped, never negative.
        let edge = crop_for((1500.0, 800.0, 500.0, 500.0), 1.25, (1920, 1080));
        assert!(edge.left <= 1918 && 1920 - edge.left - edge.right >= 2);
        assert!(edge.top <= 1078 && 1080 - edge.top - edge.bottom >= 2);
        // A sliver still records two pixels.
        let sliver = crop_for((10.0, 10.0, 0.2, 0.2), 1.0, (1920, 1080));
        assert_eq!(1920 - sliver.left - sliver.right, 2);
    }

    #[test]
    fn the_pipeline_reads_the_node_and_writes_the_file() {
        let arguments = pipeline_arguments(
            42,
            Some(Crop {
                left: 2,
                top: 4,
                right: 6,
                bottom: 8,
            }),
            Encoder::H264,
            Path::new("/home/u/Desktop/Screen Recording.mov"),
        );
        let text = arguments.join(" ");
        assert!(text.starts_with("-q -e pipewiresrc path=42 do-timestamp=true ! queue"));
        assert!(text.contains("videocrop left=2 top=4 right=6 bottom=8"));
        assert!(text.contains("x264enc speed-preset=ultrafast"));
        assert!(text.contains("! qtmux ! filesink location=/home/u/Desktop/Screen"));
        let plain = pipeline_arguments(7, None, Encoder::Vp8, Path::new("/x.webm"));
        let plain = plain.join(" ");
        assert!(!plain.contains("videocrop"));
        assert!(plain.contains("vp8enc") && plain.contains("webmmux"));
    }

    #[test]
    fn recordings_are_named_like_macos() {
        let time = chrono::NaiveDate::from_ymd_opt(2026, 10, 20)
            .unwrap()
            .and_hms_opt(11, 5, 9)
            .unwrap();
        assert_eq!(
            file_name(&time, Encoder::H264),
            "Screen Recording 2026-10-20 at 11.05.09\u{202F}AM.mov"
        );
        assert!(file_name(&time, Encoder::Vp8).ends_with(".webm"));
    }

    #[test]
    fn the_state_file_round_trips() {
        let text = state_text(1_700_000_000_000, Path::new("/d/Screen Recording.mov"));
        assert_eq!(
            parse_state(&text),
            Some((1_700_000_000_000, PathBuf::from("/d/Screen Recording.mov")))
        );
        assert_eq!(parse_state("garbage"), None);
        assert_eq!(parse_state("5\n"), None);
    }
}
