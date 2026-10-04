//! The two per-user printing preferences the Mac keeps per user, stored
//! where Linux printing already reads them, with no privilege needed:
//!
//! * the default printer: the `Default` line of `~/.cups/lpoptions`, which
//!   every CUPS client (lp, GTK's print dialog) honours before the
//!   system-wide default;
//! * the default paper size: libpaper's per-user `papersize` file
//!   (`$XDG_CONFIG_HOME/papersize`), plus a `media` option on each queue in
//!   lpoptions so CUPS clients start with that paper.
//!
//! Files are replaced atomically (write a sibling, then rename) and other
//! lines are preserved verbatim.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::model::{validate_printer_name, PaperSize};

pub struct Preferences {
    lpoptions: PathBuf,
    papersize: PathBuf,
}

impl Preferences {
    /// The signed-in user's files, or `None` without a home directory.
    pub fn for_user() -> Option<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|home| home.is_absolute())?;
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|config| config.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        Some(Self {
            lpoptions: home.join(".cups/lpoptions"),
            papersize: config.join("papersize"),
        })
    }

    pub fn at(lpoptions: impl Into<PathBuf>, papersize: impl Into<PathBuf>) -> Self {
        Self {
            lpoptions: lpoptions.into(),
            papersize: papersize.into(),
        }
    }

    /// The user's own default printer from lpoptions.
    pub fn default_printer(&self) -> Option<String> {
        let text = std::fs::read_to_string(&self.lpoptions).ok()?;
        text.lines().find_map(|line| {
            let mut words = line.split_whitespace();
            (words.next()? == "Default")
                .then(|| words.next())
                .flatten()
                .map(|name| name.split('/').next().unwrap_or(name).to_owned())
        })
    }

    /// Make `printer` the user's default: its line becomes the `Default`
    /// line, the old default goes back to `Dest`.
    pub fn set_default_printer(&self, printer: &str) -> std::io::Result<()> {
        if !validate_printer_name(printer) {
            return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
        }
        let text = std::fs::read_to_string(&self.lpoptions).unwrap_or_default();
        let mut found = false;
        let mut lines: Vec<String> = text
            .lines()
            .map(|line| {
                let mut words = line.splitn(3, char::is_whitespace);
                let kind = words.next().unwrap_or("");
                let name = words.next().unwrap_or("");
                let rest = words.next();
                if !matches!(kind, "Dest" | "Default") {
                    return line.to_owned();
                }
                let is_target = name == printer;
                found |= is_target;
                let kind = if is_target { "Default" } else { "Dest" };
                match rest {
                    Some(rest) => format!("{kind} {name} {rest}"),
                    None => format!("{kind} {name}"),
                }
            })
            .collect();
        if !found {
            lines.push(format!("Default {printer}"));
        }
        write_atomically(&self.lpoptions, &(lines.join("\n") + "\n"))
    }

    /// The user's paper size: their libpaper file, then the system one,
    /// then the locale.
    pub fn paper_size(&self) -> PaperSize {
        std::fs::read_to_string(&self.papersize)
            .ok()
            .or_else(|| std::fs::read_to_string("/etc/papersize").ok())
            .and_then(|text| {
                text.lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty() && !line.starts_with('#'))
                    .and_then(PaperSize::from_libpaper)
            })
            .unwrap_or_else(|| PaperSize::for_locale(&locale()))
    }

    /// Save the user's paper size and apply it to every listed queue.
    pub fn set_paper_size(&self, size: PaperSize, printers: &[String]) -> std::io::Result<()> {
        write_atomically(&self.papersize, &format!("{}\n", size.libpaper()))?;
        let text = std::fs::read_to_string(&self.lpoptions).unwrap_or_default();
        let mut seen: Vec<String> = Vec::new();
        let mut lines: Vec<String> = text
            .lines()
            .map(|line| {
                let mut words = line.split_whitespace();
                let kind = words.next().unwrap_or("");
                let Some(name) = words.next() else {
                    return line.to_owned();
                };
                if !matches!(kind, "Dest" | "Default") || name.contains('/') {
                    return line.to_owned();
                }
                seen.push(name.to_owned());
                let media = format!("media={}", size.cups_media());
                let mut options: Vec<&str> = words
                    .filter(|option| {
                        !option.starts_with("media=") && !option.starts_with("PageSize=")
                    })
                    .collect();
                options.push(&media);
                format!("{kind} {name} {}", options.join(" "))
            })
            .collect();
        for printer in printers {
            if validate_printer_name(printer) && !seen.contains(printer) {
                lines.push(format!("Dest {printer} media={}", size.cups_media()));
            }
        }
        write_atomically(&self.lpoptions, &(lines.join("\n") + "\n"))
    }
}

fn locale() -> String {
    ["LC_ALL", "LC_PAPER", "LANG"]
        .into_iter()
        .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty()))
        .unwrap_or_default()
}

fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("prefs"),
        std::process::id()
    ));
    {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rmac-printer-prefs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_default_printer_moves_between_lines_and_keeps_options() {
        let dir = scratch("default");
        let prefs = Preferences::at(dir.join("cups/lpoptions"), dir.join("papersize"));
        assert_eq!(prefs.default_printer(), None);
        prefs.set_default_printer("Office").unwrap();
        assert_eq!(prefs.default_printer().as_deref(), Some("Office"));
        std::fs::write(
            dir.join("cups/lpoptions"),
            "Default Office sides=two-sided-long-edge\nDest Home media=A4\nDest Home/draft print-quality=3\n",
        )
        .unwrap();
        prefs.set_default_printer("Home").unwrap();
        assert_eq!(prefs.default_printer().as_deref(), Some("Home"));
        let text = std::fs::read_to_string(dir.join("cups/lpoptions")).unwrap();
        assert_eq!(
            text,
            "Dest Office sides=two-sided-long-edge\nDefault Home media=A4\nDest Home/draft print-quality=3\n"
        );
        assert!(prefs.set_default_printer("bad name").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_paper_size_is_saved_for_libpaper_and_every_queue() {
        let dir = scratch("paper");
        let prefs = Preferences::at(dir.join("cups/lpoptions"), dir.join("config/papersize"));
        std::fs::create_dir_all(dir.join("cups")).unwrap();
        std::fs::write(
            dir.join("cups/lpoptions"),
            "Default Office media=A4 sides=one-sided\n",
        )
        .unwrap();
        prefs
            .set_paper_size(PaperSize::Letter, &["Office".into(), "Home".into()])
            .unwrap();
        assert_eq!(prefs.paper_size(), PaperSize::Letter);
        assert_eq!(
            std::fs::read_to_string(dir.join("config/papersize")).unwrap(),
            "letter\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("cups/lpoptions")).unwrap(),
            "Default Office sides=one-sided media=Letter\nDest Home media=Letter\n"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
