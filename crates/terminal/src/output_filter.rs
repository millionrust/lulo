const MAX_OSC_PAYLOAD_BYTES: usize = 1024;

#[derive(Debug, Default)]
enum State {
    #[default]
    Ground,
    Escape,
    Osc {
        bytes: Vec<u8>,
        payload_bytes: usize,
        overflowed: bool,
    },
    OscEscape {
        bytes: Vec<u8>,
        overflowed: bool,
    },
}

/// Split-safe boundary between private PTY output and VTE's growable OSC
/// storage.
#[derive(Debug, Default)]
pub(crate) struct OutputFilter {
    state: State,
    current_directory_uri: Option<String>,
    shell_marker: Option<String>,
}

impl OutputFilter {
    fn emit_osc(&mut self, bytes: &[u8], overflowed: bool, output: &mut Vec<u8>) {
        if !overflowed {
            if let Some(uri) = current_directory_uri(bytes) {
                self.current_directory_uri = Some(uri.to_owned());
            }
            if let Some(marker) = shell_marker(bytes) {
                self.shell_marker = Some(marker.to_owned());
            }
            output.extend_from_slice(bytes);
        }
    }

    pub(crate) fn take_current_directory_uri(&mut self) -> Option<String> {
        self.current_directory_uri.take()
    }

    pub(crate) fn take_shell_marker(&mut self) -> Option<String> {
        self.shell_marker.take()
    }

    fn push_osc_payload(
        bytes: &mut Vec<u8>,
        payload_bytes: &mut usize,
        overflowed: &mut bool,
        byte: u8,
    ) {
        if *overflowed {
            return;
        }
        if *payload_bytes >= MAX_OSC_PAYLOAD_BYTES {
            bytes.clear();
            *overflowed = true;
            return;
        }
        bytes.push(byte);
        *payload_bytes += 1;
    }

    /// Copy a PTY chunk into `output`, buffering OSC title/hyperlink sequences
    /// until their terminator. Bounded valid sequences are preserved
    /// byte-for-byte; overlong, malformed, and unterminated sequences never
    /// reach VTE's otherwise growable standard-library OSC buffer.
    pub(crate) fn filter_into(&mut self, input: &[u8], output: &mut Vec<u8>) {
        output.clear();
        for &byte in input {
            let state = std::mem::take(&mut self.state);
            self.state = match state {
                State::Ground if byte == 0x1b => State::Escape,
                State::Ground => {
                    output.push(byte);
                    State::Ground
                }
                State::Escape if byte == b']' => State::Osc {
                    bytes: vec![0x1b, b']'],
                    payload_bytes: 0,
                    overflowed: false,
                },
                State::Escape if matches!(byte, 0x00..=0x17 | 0x19 | 0x1c..=0x1f) => {
                    // These controls execute without leaving VTE's Escape
                    // state. Emit the state-independent control now while
                    // retaining the Escape introducer for OSC detection.
                    output.push(byte);
                    State::Escape
                }
                State::Escape if matches!(byte, 0x18 | 0x1a) => {
                    output.push(byte);
                    State::Ground
                }
                State::Escape if byte == 0x1b => {
                    output.push(0x1b);
                    State::Escape
                }
                State::Escape => {
                    output.extend_from_slice(&[0x1b, byte]);
                    State::Ground
                }
                State::Osc {
                    mut bytes,
                    payload_bytes: _,
                    overflowed,
                } if byte == 0x07 => {
                    if !overflowed {
                        bytes.push(byte);
                    }
                    self.emit_osc(&bytes, overflowed, output);
                    State::Ground
                }
                State::Osc {
                    bytes,
                    payload_bytes: _,
                    overflowed,
                } if matches!(byte, 0x18 | 0x1a) => {
                    self.emit_osc(&bytes, overflowed, output);
                    output.push(byte);
                    State::Ground
                }
                State::Osc {
                    bytes,
                    payload_bytes: _,
                    overflowed,
                } if byte == 0x1b => State::OscEscape { bytes, overflowed },
                State::Osc {
                    mut bytes,
                    mut payload_bytes,
                    mut overflowed,
                } => {
                    Self::push_osc_payload(&mut bytes, &mut payload_bytes, &mut overflowed, byte);
                    State::Osc {
                        bytes,
                        payload_bytes,
                        overflowed,
                    }
                }
                State::OscEscape {
                    mut bytes,
                    overflowed,
                } if byte == b'\\' => {
                    if !overflowed {
                        bytes.extend_from_slice(&[0x1b, b'\\']);
                    }
                    self.emit_osc(&bytes, overflowed, output);
                    State::Ground
                }
                State::OscEscape { .. } if byte == 0x1b => {
                    // The previous OSC is malformed and discarded; this new
                    // Escape can still begin a fresh, independently bounded one.
                    State::Escape
                }
                State::OscEscape { .. } => {
                    // An embedded non-ST Escape makes the OSC malformed. Drop
                    // the buffered sequence as a unit, then resume ordinary
                    // ground-state output with the current byte.
                    output.push(byte);
                    State::Ground
                }
            };
        }
    }

    #[cfg(test)]
    fn buffered_bytes(&self) -> usize {
        match &self.state {
            State::Osc { bytes, .. } | State::OscEscape { bytes, .. } => bytes.len(),
            State::Ground | State::Escape => 0,
        }
    }
}

fn current_directory_uri(bytes: &[u8]) -> Option<&str> {
    let payload = bytes
        .strip_prefix(b"\x1b]7;")?
        .strip_suffix(b"\x07")
        .or_else(|| bytes.strip_prefix(b"\x1b]7;")?.strip_suffix(b"\x1b\\"))?;
    std::str::from_utf8(payload).ok()
}

fn shell_marker(bytes: &[u8]) -> Option<&str> {
    let payload = bytes
        .strip_prefix(b"\x1b]133;")?
        .strip_suffix(b"\x07")
        .or_else(|| bytes.strip_prefix(b"\x1b]133;")?.strip_suffix(b"\x1b\\"))?;
    std::str::from_utf8(payload).ok()
}

#[cfg(test)]
mod tests {
    use super::{OutputFilter, MAX_OSC_PAYLOAD_BYTES};

    #[test]
    fn preserves_normal_unicode_and_split_valid_osc() {
        let chunks: &[&[u8]] = &[
            b"plain \xce",
            b"\xbb \x1b",
            b"]0;Private-safe title",
            b"\x1b",
            b"\\ tail \x1b[31mred",
        ];
        let mut filter = OutputFilter::default();
        let mut scratch = Vec::new();
        let mut output = Vec::new();
        for chunk in chunks {
            filter.filter_into(chunk, &mut scratch);
            output.extend_from_slice(&scratch);
        }

        assert_eq!(
            output,
            b"plain \xce\xbb \x1b]0;Private-safe title\x1b\\ tail \x1b[31mred"
        );
        assert_eq!(filter.buffered_bytes(), 0);
    }

    #[test]
    fn drops_overlong_and_malformed_but_preserves_bounded_hyperlink_osc() {
        let mut filter = OutputFilter::default();
        let mut scratch = Vec::new();
        let mut output = Vec::new();

        let mut exact_prefix = b"before\x1b]0;".to_vec();
        exact_prefix.resize(
            exact_prefix.len() + MAX_OSC_PAYLOAD_BYTES.saturating_sub(2),
            b'a',
        );
        filter.filter_into(&exact_prefix, &mut scratch);
        output.extend_from_slice(&scratch);
        assert_eq!(output, b"before");
        assert_eq!(
            filter.buffered_bytes(),
            MAX_OSC_PAYLOAD_BYTES + b"\x1b]".len()
        );

        filter.filter_into(b"x\x07after", &mut scratch);
        output.extend_from_slice(&scratch);
        assert_eq!(output, b"beforeafter");
        assert_eq!(filter.buffered_bytes(), 0);

        filter.filter_into(b"\x1b]8;;https://example.invalid\x1b\\safe", &mut scratch);
        assert_eq!(scratch, b"\x1b]8;;https://example.invalid\x1b\\safe");
        filter.filter_into(
            b"\x1b]\x008;;https://example.invalid\x07still-safe",
            &mut scratch,
        );
        assert_eq!(
            scratch,
            b"\x1b]\x008;;https://example.invalid\x07still-safe"
        );

        filter.filter_into(b"\x1b]0;malformed\x1bXresumed", &mut scratch);
        assert_eq!(scratch, b"Xresumed");
        assert_eq!(filter.buffered_bytes(), 0);

        // C0 controls do not leave VTE's Escape state, so they must not allow
        // a split `ESC <control> ]` sequence to bypass the OSC limit.
        let mut bypass_filter = OutputFilter::default();
        bypass_filter.filter_into(b"\x1b\x07", &mut scratch);
        assert_eq!(scratch, b"\x07");
        let mut bypass = b"]0;".to_vec();
        bypass.resize(bypass.len() + MAX_OSC_PAYLOAD_BYTES + 1, b'b');
        bypass.extend_from_slice(b"\x07safe");
        bypass_filter.filter_into(&bypass, &mut scratch);
        assert_eq!(scratch, b"safe");
        assert_eq!(bypass_filter.buffered_bytes(), 0);
    }

    #[test]
    fn accepts_the_exact_payload_limit_and_bel() {
        let mut sequence = b"\x1b]2;".to_vec();
        sequence.resize(
            sequence.len() + MAX_OSC_PAYLOAD_BYTES.saturating_sub(2),
            b't',
        );
        sequence.push(0x07);

        let mut filter = OutputFilter::default();
        let mut output = Vec::new();
        filter.filter_into(&sequence, &mut output);

        assert_eq!(output, sequence);
        assert_eq!(filter.buffered_bytes(), 0);
    }

    #[test]
    fn reports_only_complete_bounded_osc_7_values() {
        let mut filter = OutputFilter::default();
        let mut output = Vec::new();

        filter.filter_into(b"\x1b]7;file:///home/jacob/Pro", &mut output);
        assert!(output.is_empty());
        assert_eq!(filter.take_current_directory_uri(), None);

        filter.filter_into(b"jects/rmac\x1b\\prompt", &mut output);
        assert_eq!(
            output,
            b"\x1b]7;file:///home/jacob/Projects/rmac\x1b\\prompt"
        );
        assert_eq!(
            filter.take_current_directory_uri().as_deref(),
            Some("file:///home/jacob/Projects/rmac")
        );
        assert_eq!(filter.take_current_directory_uri(), None);

        let mut overlong = b"\x1b]7;file:///".to_vec();
        overlong.resize(overlong.len() + MAX_OSC_PAYLOAD_BYTES, b'a');
        overlong.push(0x07);
        filter.filter_into(&overlong, &mut output);
        assert!(output.is_empty());
        assert_eq!(filter.take_current_directory_uri(), None);

        filter.filter_into(b"\x1b]7;file:///private\x1bXplain", &mut output);
        assert_eq!(output, b"Xplain");
        assert_eq!(filter.take_current_directory_uri(), None);
    }

    #[test]
    fn reports_only_complete_bounded_osc_133_markers() {
        let mut filter = OutputFilter::default();
        let mut output = Vec::new();

        filter.filter_into(b"\x1b]133;", &mut output);
        assert!(output.is_empty());
        assert_eq!(filter.take_shell_marker(), None);

        filter.filter_into(b"C\x1b\\running", &mut output);
        assert_eq!(output, b"\x1b]133;C\x1b\\running");
        assert_eq!(filter.take_shell_marker().as_deref(), Some("C"));

        filter.filter_into(b"\x1b]133;D;7\x07\x1b]133;A\x07prompt", &mut output);
        assert_eq!(filter.take_shell_marker().as_deref(), Some("A"));
        assert_eq!(filter.take_shell_marker(), None);

        filter.filter_into(b"\x1b]133;D;private\x1bXplain", &mut output);
        assert_eq!(output, b"Xplain");
        assert_eq!(filter.take_shell_marker(), None);
    }
}
