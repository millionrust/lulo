//! Lulo apps' notifications on Windows (ADR 0023, "Notification Centre and
//! the app switcher on Windows").
//!
//! On Lulo OS an app posts through `org.freedesktop.Notifications` on the
//! session bus. Windows has no session bus, so the Notification Centre
//! that `lulo-shell` hosts serves `\\.\pipe\lulo-notifications-<user>`
//! (no remote clients, the default same-user security) and a Lulo app
//! writes one notification per connection:
//!
//! ```text
//! app → centre   post <TAB> summary <TAB> body <TAB> actions <TAB> urgency <TAB> flags
//! centre → app   posted <TAB> id
//! centre → app   action <TAB> key          (zero or more)
//! centre → app   closed <TAB> reason       (last)
//! ```
//!
//! `actions` is the freedesktop list (key, label, key, label …) joined by
//! U+001F; `flags` is a set of letters: `r` resident, `t` transient, `s`
//! suppress sound. Fields carry no control characters. The app writes only
//! its first line and then only reads, and the centre only writes after
//! reading it, so neither side ever reads and writes one synchronous pipe
//! handle at the same time. Who posted comes from the pipe itself (the
//! client's process), never from the message. Closing the connection does
//! not withdraw the notification; it stays in the Centre's history.

/// A longer line is refused (the summary, body and eight actions fit well
/// inside it).
pub const MAX_LINE_BYTES: usize = 64 * 1024;
const FIELD: char = '\t';
const LIST: char = '\u{1f}';

/// `\\.\pipe\lulo-notifications-<user>`, one per signed-in user.
pub fn pipe_name(user: &str) -> String {
    format!(r"\\.\pipe\lulo-notifications-{}", sanitized_user(user))
}

fn sanitized_user(user: &str) -> String {
    user.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect()
}

/// One notification a Lulo app posts.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Post {
    pub summary: String,
    pub body: String,
    /// Freedesktop action pairs: `(key, label)`; `default` is the click.
    pub actions: Vec<(String, String)>,
    /// Freedesktop urgency: 0 low, 1 normal, 2 critical.
    pub urgency: u8,
    pub resident: bool,
    pub transient: bool,
    pub suppress_sound: bool,
}

/// What the centre answers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Answer {
    /// The notification's id.
    Posted(u32),
    /// The user chose the action with this key.
    Action(String),
    /// The notification closed, with the freedesktop reason (1 expired, 2
    /// dismissed, 3 withdrawn).
    Closed(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Malformed,
    TooLong,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Malformed => "malformed notification pipe line",
            Self::TooLong => "notification pipe line is too long",
        })
    }
}

impl std::error::Error for Error {}

/// Control characters become spaces, so no field can end a field or the
/// line early.
fn clean(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

pub fn encode_post(post: &Post) -> Result<String, Error> {
    let actions = post
        .actions
        .iter()
        .flat_map(|(key, label)| [clean(key), clean(label)])
        .collect::<Vec<_>>()
        .join(&LIST.to_string());
    let mut flags = String::new();
    if post.resident {
        flags.push('r');
    }
    if post.transient {
        flags.push('t');
    }
    if post.suppress_sound {
        flags.push('s');
    }
    let line = format!(
        "post{FIELD}{}{FIELD}{}{FIELD}{actions}{FIELD}{}{FIELD}{flags}\n",
        clean(&post.summary),
        clean(&post.body),
        post.urgency.min(2),
    );
    if line.len() > MAX_LINE_BYTES {
        return Err(Error::TooLong);
    }
    Ok(line)
}

pub fn decode_post(line: &str) -> Result<Post, Error> {
    if line.len() > MAX_LINE_BYTES {
        return Err(Error::TooLong);
    }
    let fields = line.split(FIELD).collect::<Vec<_>>();
    let [kind, summary, body, actions, urgency, flags] = fields.as_slice() else {
        return Err(Error::Malformed);
    };
    if *kind != "post" {
        return Err(Error::Malformed);
    }
    let list = if actions.is_empty() {
        Vec::new()
    } else {
        actions.split(LIST).collect::<Vec<_>>()
    };
    if !list.len().is_multiple_of(2) {
        return Err(Error::Malformed);
    }
    let urgency = urgency.parse::<u8>().map_err(|_| Error::Malformed)?;
    if urgency > 2 || flags.chars().any(|flag| !matches!(flag, 'r' | 't' | 's')) {
        return Err(Error::Malformed);
    }
    Ok(Post {
        summary: (*summary).to_owned(),
        body: (*body).to_owned(),
        actions: list
            .chunks_exact(2)
            .map(|pair| (pair[0].to_owned(), pair[1].to_owned()))
            .collect(),
        urgency,
        resident: flags.contains('r'),
        transient: flags.contains('t'),
        suppress_sound: flags.contains('s'),
    })
}

pub fn encode_answer(answer: &Answer) -> String {
    match answer {
        Answer::Posted(id) => format!("posted{FIELD}{id}\n"),
        Answer::Action(key) => format!("action{FIELD}{}\n", clean(key)),
        Answer::Closed(reason) => format!("closed{FIELD}{reason}\n"),
    }
}

pub fn decode_answer(line: &str) -> Result<Answer, Error> {
    let (kind, value) = line.split_once(FIELD).ok_or(Error::Malformed)?;
    match kind {
        "posted" => value
            .parse()
            .map(Answer::Posted)
            .map_err(|_| Error::Malformed),
        "action" if !value.is_empty() => Ok(Answer::Action(value.to_owned())),
        "closed" => value
            .parse()
            .map(Answer::Closed)
            .map_err(|_| Error::Malformed),
        _ => Err(Error::Malformed),
    }
}

/// Post `post` to the Notification Centre `lulo-shell` hosts. Returns the
/// notification's id and the centre's later answers (the action chosen,
/// then its closing), read on a thread of their own; `None` when no Lulo
/// layer is running or it refused the post. Blocking.
#[cfg(windows)]
pub fn post(post: &Post) -> Option<(u32, std::sync::mpsc::Receiver<Answer>)> {
    use std::io::{BufRead as _, BufReader, Write as _};

    let user = std::env::var("USERNAME").unwrap_or_default();
    let mut pipe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(pipe_name(&user))
        .ok()?;
    pipe.write_all(encode_post(post).ok()?.as_bytes()).ok()?;
    let mut reader = BufReader::new(pipe);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let Ok(Answer::Posted(id)) = decode_answer(line.trim_end_matches('\n')) else {
        return None;
    };
    let (sender, answers) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("lulo-notification-answers".into())
        .spawn(move || loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) if line.len() > MAX_LINE_BYTES => return,
                Ok(_) => {
                    let Ok(answer) = decode_answer(line.trim_end_matches('\n')) else {
                        continue;
                    };
                    let closed = matches!(answer, Answer::Closed(_));
                    if sender.send(answer).is_err() || closed {
                        return;
                    }
                }
            }
        })
        .ok()?;
    Some((id, answers))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alarm() -> Post {
        Post {
            summary: "Wake\tup".into(),
            body: "7:00\nAM".into(),
            actions: vec![
                ("default".into(), "Open".into()),
                ("stop".into(), "Stop".into()),
            ],
            urgency: 2,
            resident: true,
            transient: false,
            suppress_sound: true,
        }
    }

    #[test]
    fn a_post_round_trips_through_one_line() {
        let line = encode_post(&alarm()).unwrap();
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        let decoded = decode_post(line.trim_end_matches('\n')).unwrap();
        // Control characters in text become spaces.
        assert_eq!(decoded.summary, "Wake up");
        assert_eq!(decoded.body, "7:00 AM");
        assert_eq!(decoded.actions, alarm().actions);
        assert_eq!(decoded.urgency, 2);
        assert!(decoded.resident && decoded.suppress_sound && !decoded.transient);
    }

    #[test]
    fn malformed_posts_are_refused() {
        assert_eq!(decode_post("post\ta\tb"), Err(Error::Malformed));
        assert_eq!(decode_post("hello\ta\tb\t\t1\t"), Err(Error::Malformed));
        assert_eq!(
            decode_post("post\ta\tb\tonly-a-key\t1\t"),
            Err(Error::Malformed)
        );
        assert_eq!(decode_post("post\ta\tb\t\t7\t"), Err(Error::Malformed));
        assert_eq!(decode_post("post\ta\tb\t\t1\tx"), Err(Error::Malformed));
        let long = format!("post\t{}\tb\t\t1\t", "a".repeat(MAX_LINE_BYTES));
        assert_eq!(decode_post(&long), Err(Error::TooLong));
        assert!(decode_post("post\t\t\t\t0\t").is_ok());
    }

    #[test]
    fn answers_round_trip() {
        for answer in [
            Answer::Posted(42),
            Answer::Action("snooze".into()),
            Answer::Closed(2),
        ] {
            let line = encode_answer(&answer);
            assert_eq!(decode_answer(line.trim_end_matches('\n')), Ok(answer));
        }
        assert_eq!(decode_answer("posted\tx"), Err(Error::Malformed));
        assert_eq!(decode_answer("action\t"), Err(Error::Malformed));
        assert_eq!(decode_answer("nothing"), Err(Error::Malformed));
    }

    #[test]
    fn pipe_names_carry_only_safe_user_characters() {
        assert_eq!(
            pipe_name(r"ru ban\..x"),
            r"\\.\pipe\lulo-notifications-ruban..x"
        );
    }
}
