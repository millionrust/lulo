//! Platform-neutral printer, job, device and paper-size model.

use crate::ipp::Group;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PrinterState {
    #[default]
    Idle,
    Printing,
    Stopped,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Printer {
    /// The CUPS queue name (no spaces); what every operation addresses.
    pub name: String,
    /// `printer-info`: the name people see.
    pub info: String,
    pub location: String,
    pub make_and_model: String,
    pub state: PrinterState,
    pub reasons: Vec<String>,
    pub accepting: bool,
    pub shared: bool,
    pub device_uri: String,
    pub is_class: bool,
}

impl Printer {
    pub fn display_name(&self) -> &str {
        if self.info.trim().is_empty() {
            &self.name
        } else {
            &self.info
        }
    }

    pub fn is_offline(&self) -> bool {
        self.reasons.iter().any(|reason| {
            reason.starts_with("offline") || reason.starts_with("connecting-to-device")
        })
    }

    pub fn is_paused(&self) -> bool {
        self.state == PrinterState::Stopped || self.reasons.iter().any(|reason| reason == "paused")
    }

    /// The Mac's one-word status under a printer's name.
    pub fn status_label(&self) -> &'static str {
        if self.is_paused() {
            "Paused"
        } else if self.is_offline() {
            "Offline"
        } else if self.state == PrinterState::Printing {
            "Printing"
        } else {
            "Idle"
        }
    }

    pub(crate) fn from_group(group: &Group) -> Option<Self> {
        let name = group.text("printer-name")?.to_owned();
        let printer_type = group.integer("printer-type").unwrap_or(0);
        Some(Self {
            name,
            info: group.text("printer-info").unwrap_or_default().to_owned(),
            location: group
                .text("printer-location")
                .unwrap_or_default()
                .to_owned(),
            make_and_model: group
                .text("printer-make-and-model")
                .unwrap_or_default()
                .to_owned(),
            state: match group.integer("printer-state") {
                Some(4) => PrinterState::Printing,
                Some(5) => PrinterState::Stopped,
                _ => PrinterState::Idle,
            },
            reasons: group
                .texts("printer-state-reasons")
                .into_iter()
                .filter(|reason| *reason != "none")
                .map(|reason| {
                    reason
                        .trim_end_matches("-report")
                        .trim_end_matches("-warning")
                        .trim_end_matches("-error")
                        .to_owned()
                })
                .collect(),
            accepting: group.boolean("printer-is-accepting-jobs").unwrap_or(true),
            shared: group.boolean("printer-is-shared").unwrap_or(false),
            device_uri: group.text("device-uri").unwrap_or_default().to_owned(),
            // CUPS_PRINTER_CLASS | CUPS_PRINTER_IMPLICIT
            is_class: printer_type & 0x0001 != 0,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum JobState {
    #[default]
    Pending,
    Held,
    Printing,
    Stopped,
    Canceled,
    Aborted,
    Completed,
}

impl JobState {
    fn from_ipp(value: i32) -> Self {
        match value {
            4 => Self::Held,
            5 => Self::Printing,
            6 => Self::Stopped,
            7 => Self::Canceled,
            8 => Self::Aborted,
            9 => Self::Completed,
            _ => Self::Pending,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "Waiting",
            Self::Held => "On Hold",
            Self::Printing => "Printing",
            Self::Stopped => "Stopped",
            Self::Canceled => "Cancelled",
            Self::Aborted => "Aborted",
            Self::Completed => "Completed",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Job {
    pub id: i32,
    pub name: String,
    pub owner: String,
    pub state: JobState,
    pub size_kb: i32,
}

impl Job {
    pub(crate) fn from_group(group: &Group) -> Option<Self> {
        Some(Self {
            id: group.integer("job-id")?,
            name: group.text("job-name").unwrap_or("Untitled").to_owned(),
            owner: group
                .text("job-originating-user-name")
                .unwrap_or_default()
                .to_owned(),
            state: JobState::from_ipp(group.integer("job-state").unwrap_or(3)),
            size_kb: group.integer("job-k-octets").unwrap_or(0),
        })
    }
}

/// A printer CUPS found on the network or a local port.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Device {
    pub uri: String,
    pub info: String,
    pub make_and_model: String,
    pub location: String,
}

impl Device {
    /// Whether CUPS can build the queue itself from the printer's IPP
    /// description (IPP Everywhere / AirPrint / Mopria), with no driver.
    pub fn is_driverless(&self) -> bool {
        let uri = self.uri.to_ascii_lowercase();
        uri.starts_with("ipp://")
            || uri.starts_with("ipps://")
            || uri.starts_with("ippusb://")
            || (uri.starts_with("dnssd://") && uri.contains("._ipp"))
    }

    /// How the Mac's Add Printer list labels the connection.
    pub fn kind(&self) -> &'static str {
        let uri = self.uri.to_ascii_lowercase();
        if uri.starts_with("dnssd://") {
            "Bonjour"
        } else if uri.starts_with("ippusb://") || uri.starts_with("usb://") {
            "USB"
        } else {
            "IP"
        }
    }

    pub fn display_name(&self) -> &str {
        if self.info.trim().is_empty() {
            &self.uri
        } else {
            &self.info
        }
    }
}

/// Parse cups-pk-helper's flattened `DevicesGet` dictionary
/// (`device-uri:0`, `device-info:0`, …) into devices CUPS can add without
/// a driver, de-duplicated by name.
pub fn devices_from_flat(values: &std::collections::HashMap<String, String>) -> Vec<Device> {
    let mut by_index: std::collections::BTreeMap<u32, Device> = Default::default();
    for (key, value) in values {
        let Some((attribute, index)) = key.rsplit_once(':') else {
            continue;
        };
        let Ok(index) = index.parse::<u32>() else {
            continue;
        };
        let device = by_index.entry(index).or_default();
        match attribute {
            "device-uri" => device.uri = value.clone(),
            "device-info" => device.info = value.clone(),
            "device-make-and-model" => device.make_and_model = value.clone(),
            "device-location" => device.location = value.clone(),
            _ => {}
        }
    }
    let mut devices: Vec<Device> = Vec::new();
    for device in by_index.into_values() {
        if !device.is_driverless() {
            continue;
        }
        // Prefer the encrypted or Bonjour entry when one printer is listed
        // more than once.
        if let Some(existing) = devices
            .iter_mut()
            .find(|existing| existing.display_name() == device.display_name())
        {
            if device.uri.starts_with("ipps://") || device.uri.starts_with("dnssd://") {
                *existing = device;
            }
            continue;
        }
        devices.push(device);
    }
    devices.sort_by(|left, right| {
        left.display_name()
            .to_lowercase()
            .cmp(&right.display_name().to_lowercase())
    });
    devices
}

/// CUPS queue names: 1–127 printable characters, no space, tab, slash,
/// backslash, hash, question mark, quotes or comma.
pub fn validate_printer_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 127
        && name.chars().all(|c| {
            c.is_ascii_graphic() && !matches!(c, '/' | '\\' | '#' | '?' | '\'' | '"' | ',')
        })
}

/// The queue name the Add sheet proposes from a printer's display name:
/// unsupported characters become underscores.
pub fn suggest_printer_name(info: &str) -> String {
    let mut name: String = info
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && !matches!(c, '/' | '\\' | '#' | '?' | '\'' | '"' | ',') {
                c
            } else {
                '_'
            }
        })
        .collect();
    while name.contains("__") {
        name = name.replace("__", "_");
    }
    let name = name.trim_matches('_');
    let mut name: String = name.chars().take(127).collect();
    if name.is_empty() {
        name.push_str("Printer");
    }
    name
}

/// The paper sizes the Default paper size pop-up offers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaperSize {
    A3,
    A4,
    A5,
    Letter,
    Legal,
}

impl PaperSize {
    pub const ALL: [PaperSize; 5] = [
        PaperSize::A3,
        PaperSize::A4,
        PaperSize::A5,
        PaperSize::Letter,
        PaperSize::Legal,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PaperSize::A3 => "A3",
            PaperSize::A4 => "A4",
            PaperSize::A5 => "A5",
            PaperSize::Letter => "US Letter",
            PaperSize::Legal => "US Legal",
        }
    }

    /// libpaper's name, written to the papersize file.
    pub fn libpaper(self) -> &'static str {
        match self {
            PaperSize::A3 => "a3",
            PaperSize::A4 => "a4",
            PaperSize::A5 => "a5",
            PaperSize::Letter => "letter",
            PaperSize::Legal => "legal",
        }
    }

    /// The CUPS `media` keyword, written to lpoptions.
    pub fn cups_media(self) -> &'static str {
        match self {
            PaperSize::A3 => "A3",
            PaperSize::A4 => "A4",
            PaperSize::A5 => "A5",
            PaperSize::Letter => "Letter",
            PaperSize::Legal => "Legal",
        }
    }

    pub fn from_libpaper(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|size| size.libpaper() == name || size.cups_media().to_ascii_lowercase() == name)
    }

    /// The locale's paper: Letter in North and Central America and the
    /// Philippines, A4 elsewhere (glibc's LC_PAPER data).
    pub fn for_locale(locale: &str) -> Self {
        let region = locale
            .split(['.', '@'])
            .next()
            .and_then(|tag| tag.split('_').nth(1))
            .unwrap_or("");
        if matches!(
            region,
            "US" | "CA"
                | "MX"
                | "PR"
                | "PH"
                | "CL"
                | "CO"
                | "VE"
                | "CR"
                | "GT"
                | "NI"
                | "PA"
                | "SV"
                | "DO"
                | "BZ"
        ) {
            PaperSize::Letter
        } else {
            PaperSize::A4
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipp::tests::ResponseBuilder;
    use crate::ipp::{
        parse, TAG_BOOLEAN, TAG_ENUM, TAG_INTEGER, TAG_KEYWORD, TAG_NAME, TAG_TEXT, TAG_URI,
    };

    #[test]
    fn printers_read_status_from_state_and_reasons() {
        let body = ResponseBuilder::new(0)
            .group(0x04)
            .text(TAG_NAME, "printer-name", "Office_Laser")
            .text(TAG_TEXT, "printer-info", "Office Laser")
            .int(TAG_ENUM, "printer-state", 3)
            .text(TAG_KEYWORD, "printer-state-reasons", "offline-report")
            .raw(TAG_BOOLEAN, "printer-is-accepting-jobs", &[1])
            .text(TAG_URI, "device-uri", "ipp://laser.local/ipp/print")
            .int(TAG_ENUM, "printer-type", 0x0004)
            .group(0x04)
            .text(TAG_NAME, "printer-name", "Paused")
            .int(TAG_ENUM, "printer-state", 5)
            .text(TAG_KEYWORD, "printer-state-reasons", "paused")
            .finish();
        let response = parse(&body).unwrap();
        let printers: Vec<_> = response
            .groups_tagged(0x04)
            .filter_map(Printer::from_group)
            .collect();
        assert_eq!(printers[0].display_name(), "Office Laser");
        assert_eq!(printers[0].status_label(), "Offline");
        assert_eq!(printers[0].reasons, ["offline"]);
        assert!(!printers[0].is_class);
        assert_eq!(printers[1].display_name(), "Paused");
        assert_eq!(printers[1].status_label(), "Paused");
    }

    #[test]
    fn jobs_read_their_state() {
        let body = ResponseBuilder::new(0)
            .group(0x02)
            .int(TAG_INTEGER, "job-id", 12)
            .text(TAG_NAME, "job-name", "Report.pdf")
            .text(TAG_NAME, "job-originating-user-name", "amy")
            .int(TAG_ENUM, "job-state", 5)
            .int(TAG_INTEGER, "job-k-octets", 40)
            .finish();
        let response = parse(&body).unwrap();
        let job = response
            .groups_tagged(0x02)
            .find_map(Job::from_group)
            .unwrap();
        assert_eq!(job.id, 12);
        assert_eq!(job.state.label(), "Printing");
        assert_eq!(job.owner, "amy");
    }

    #[test]
    fn only_driverless_devices_are_offered_once_each() {
        let flat: std::collections::HashMap<String, String> = [
            (
                "device-uri:0",
                "dnssd://Office%20Laser._ipp._tcp.local/?uuid=1",
            ),
            ("device-info:0", "Office Laser"),
            ("device-uri:1", "ipp://192.168.1.20/ipp/print"),
            ("device-info:1", "Office Laser"),
            ("device-uri:2", "usb://Old/Printer"),
            ("device-info:2", "Old Printer"),
            ("device-uri:3", "ipps://photo.local/ipp/print"),
            ("device-info:3", "Photo"),
            ("device-uri:4", "socket://10.0.0.9"),
            ("device-info:4", "Raw socket"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
        let devices = devices_from_flat(&flat);
        let names: Vec<_> = devices.iter().map(|device| device.display_name()).collect();
        assert_eq!(names, ["Office Laser", "Photo"]);
        assert!(devices[0].uri.starts_with("dnssd://"));
        assert_eq!(devices[0].kind(), "Bonjour");
    }

    #[test]
    fn printer_names_follow_cups_rules() {
        assert!(validate_printer_name("Office_Laser"));
        assert!(!validate_printer_name("Office Laser"));
        assert!(!validate_printer_name("a/b"));
        assert!(!validate_printer_name(""));
        assert!(!validate_printer_name(&"a".repeat(128)));
        assert_eq!(suggest_printer_name("HP LaserJet / 400"), "HP_LaserJet_400");
        assert!(validate_printer_name(&suggest_printer_name("Büro Drucker")));
        assert_eq!(suggest_printer_name("   "), "Printer");
    }

    #[test]
    fn paper_sizes_follow_the_locale_and_round_trip() {
        assert_eq!(PaperSize::for_locale("en_US.UTF-8"), PaperSize::Letter);
        assert_eq!(PaperSize::for_locale("en_GB.UTF-8"), PaperSize::A4);
        assert_eq!(PaperSize::for_locale("C"), PaperSize::A4);
        for size in PaperSize::ALL {
            assert_eq!(PaperSize::from_libpaper(size.libpaper()), Some(size));
        }
        assert_eq!(
            PaperSize::from_libpaper("Letter\n"),
            Some(PaperSize::Letter)
        );
    }
}
