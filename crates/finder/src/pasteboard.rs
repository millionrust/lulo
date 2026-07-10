//! Native macOS pasteboard bridge for files.
//!
//! Copying in rmac Finder writes real `file://` URLs to the general pasteboard
//! (`NSPasteboard`), so the system Finder — and any other app — can paste the
//! copied items. Likewise, paste reads file URLs the real Finder put there.
//! This is the genuine system clipboard, not the newline-joined text fallback.

#[cfg(target_os = "macos")]
mod imp {
    use std::path::PathBuf;

    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2::ClassType;
    use objc2_app_kit::{NSPasteboard, NSPasteboardWriting};
    use objc2_foundation::{NSArray, NSString, NSURL};

    /// Write the given file paths to the general pasteboard as `file://` URLs.
    pub fn write_file_urls(paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }
        let objs: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = paths
            .iter()
            .map(|p| {
                let s = NSString::from_str(&p.to_string_lossy());
                let url = NSURL::fileURLWithPath(&s);
                ProtocolObject::from_retained(url)
            })
            .collect();
        let array = NSArray::from_retained_slice(&objs);
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        pb.writeObjects(&array);
    }

    /// Read any `file://` URLs currently on the general pasteboard.
    pub fn read_file_urls() -> Vec<PathBuf> {
        let pb = NSPasteboard::generalPasteboard();
        let classes = NSArray::from_slice(&[NSURL::class()]);
        // SAFETY: `classes` holds the `NSURL` class and no read options are
        // passed, matching the documented contract.
        let Some(objs) = (unsafe { pb.readObjectsForClasses_options(&classes, None) }) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for obj in objs.iter() {
            if let Ok(url) = obj.downcast::<NSURL>() {
                if let Some(path) = url.path() {
                    out.push(PathBuf::from(path.to_string()));
                }
            }
        }
        out
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use std::path::PathBuf;
    pub fn write_file_urls(_paths: &[PathBuf]) {}
    pub fn read_file_urls() -> Vec<PathBuf> {
        Vec::new()
    }
}

pub use imp::{read_file_urls, write_file_urls};

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn round_trips_file_urls_through_the_system_pasteboard() {
        // Two real paths that exist on every macOS box.
        let want = vec![PathBuf::from("/tmp"), PathBuf::from("/usr/bin/true")];
        write_file_urls(&want);
        let got = read_file_urls();
        // The pasteboard normalises /tmp → /private/tmp; compare by file name +
        // existence rather than exact string.
        assert_eq!(got.len(), want.len(), "got {got:?}");
        assert!(
            got.iter().all(|p| p.exists()),
            "all read paths exist: {got:?}"
        );
        assert!(got.iter().any(|p| p.ends_with("true")));
    }
}
