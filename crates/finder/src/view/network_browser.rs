//! The sidebar's Network location (FILES-07): the servers already connected
//! (their GVFS folders, which open like any folder) followed by the
//! computers and shares GVFS finds on the local network (`network:///`,
//! SMB browsing and DNS-SD). Opening a found computer connects to it
//! through Connect to Server, which asks for a name and password when the
//! server wants them.
//!
//! Windows has no GVFS; its Network row is left out there.

use super::*;

/// How long one network browse may take before Files shows what it has.
#[cfg(target_os = "linux")]
const BROWSE_SECONDS: &str = "6";

/// A computer or share GVFS found, by name, with the address to connect to.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NetworkHost {
    pub(super) name: String,
    pub(super) uri: String,
}

/// Parse `gio list -l -a standard::display-name,standard::target-uri`:
/// one tab-separated line per item, its attributes as `key=value` fields.
#[cfg(any(target_os = "linux", test))]
pub(super) fn parse_gio_list(output: &str) -> Vec<NetworkHost> {
    let mut hosts = Vec::new();
    for line in output.lines() {
        let fields = line.split('\t').collect::<Vec<_>>();
        let Some(raw_name) = fields.first().map(|name| name.trim()) else {
            continue;
        };
        if raw_name.is_empty() {
            continue;
        }
        let attribute = |key: &str| {
            fields
                .iter()
                .find_map(|field| field.trim().strip_prefix(key))
                .map(str::trim)
                .filter(|value| !value.is_empty())
        };
        let Some(uri) = attribute("standard::target-uri=") else {
            continue;
        };
        if !network_uri(uri) {
            continue;
        }
        let name = attribute("standard::display-name=").unwrap_or(raw_name);
        let host = NetworkHost {
            name: name.to_owned(),
            uri: uri.to_owned(),
        };
        if !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    hosts.sort_by_key(|host| host.name.to_lowercase());
    hosts
}

/// The schemes Connect to Server can mount.
fn network_uri(uri: &str) -> bool {
    ["smb://", "sftp://", "ftp://", "dav://", "davs://", "afp://", "nfs://"]
        .iter()
        .any(|scheme| uri.starts_with(scheme))
}

/// A found computer is listed under its address, which is never an
/// absolute path, so it cannot be mistaken for a real item.
pub(super) fn network_address(path: &Path) -> Option<String> {
    if path.is_absolute() {
        return None;
    }
    let text = path.to_str()?;
    network_uri(text).then(|| text.to_owned())
}

fn host_entry(host: &NetworkHost) -> Entry {
    Entry {
        name: host.name.clone().into(),
        path: PathBuf::from(&host.uri),
        is_dir: false,
        size: "--".into(),
        modified: "--".into(),
        modified_absolute: "--".into(),
        created: "--".into(),
        created_absolute: "--".into(),
        last_opened: "--".into(),
        last_opened_absolute: "--".into(),
        added: "--".into(),
        added_absolute: "--".into(),
        kind: "Server".into(),
        size_bytes: 0,
        mtime: SystemTime::UNIX_EPOCH,
        created_time: SystemTime::UNIX_EPOCH,
        last_opened_time: SystemTime::UNIX_EPOCH,
        tag: None,
        comment: None,
        search_detail: Some(host.uri.clone().into()),
        application: None,
    }
}

#[cfg(target_os = "linux")]
fn browse_network() -> Vec<NetworkHost> {
    // `timeout` bounds a browse that waits on an unresponsive host.
    let output = std::process::Command::new("timeout")
        .args([
            BROWSE_SECONDS,
            "gio",
            "list",
            "-l",
            "-a",
            "standard::display-name,standard::target-uri",
            "network:///",
        ])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output();
    match output {
        Ok(output) => parse_gio_list(&String::from_utf8_lossy(&output.stdout)),
        Err(_) => Vec::new(),
    }
}

/// Every connected server's folder under `$XDG_RUNTIME_DIR/gvfs`.
#[cfg(target_os = "linux")]
fn connected_servers() -> Vec<Entry> {
    let Some(root) = std::env::var_os("XDG_RUNTIME_DIR").map(|dir| PathBuf::from(dir).join("gvfs"))
    else {
        return Vec::new();
    };
    let Ok(children) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut entries = children
        .flatten()
        .filter_map(|child| entry_for(&child.path()))
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.name.to_lowercase());
    entries
}

impl FinderView {
    /// The sidebar's Network row.
    pub(super) fn network_click(&mut self, cx: &mut Context<Self>) {
        self.trash_view = false;
        self.applications_view = false;
        self.result_title = Some("Network".into());
        let (generation, cancel) = self.begin_search();
        self.entries.clear();
        self.selected.clear();
        self.anchor = None;
        self.search_summary = Some("Looking for computers on the network…".into());
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            #[cfg(target_os = "linux")]
            let (connected, found) = blocking::unblock(move || {
                let connected = connected_servers();
                let found = if cancel.load(Ordering::Acquire) {
                    Vec::new()
                } else {
                    browse_network()
                };
                (connected, found)
            })
            .await;
            #[cfg(not(target_os = "linux"))]
            let (connected, found) = {
                let _ = cancel;
                (Vec::<Entry>::new(), Vec::<NetworkHost>::new())
            };
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                if this.search_generation != generation {
                    return;
                }
                this.search_cancel = None;
                let count = connected.len() + found.len();
                this.entries = connected;
                this.entries.extend(found.iter().map(host_entry));
                this.search_summary = Some(
                    match count {
                        0 => "No computers were found on the network".to_owned(),
                        1 => "1 item".to_owned(),
                        count => format!("{count} items"),
                    }
                    .into(),
                );
                this.selected.clear();
                this.anchor = None;
                cx.notify();
            });
        })
        .detach();
    }

    /// Opening a found computer connects to it; the window shows the share
    /// once it is mounted.
    pub(super) fn connect_to_network_address(&mut self, uri: String, cx: &mut Context<Self>) {
        self.pending_connect = Some(uri);
        cx.notify();
    }

    /// Start a connection requested where no window was at hand.
    pub(super) fn start_pending_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(uri) = self.pending_connect.take() else {
            return;
        };
        self.open_connect_to_server(window, cx);
        self.set_connect_address(&uri, window, cx);
        self.start_connect(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gio_listings_become_named_addresses() {
        let output = "smb-root\t0\t(mountable)\tstandard::display-name=Windows Network\tstandard::target-uri=smb:///\n\
                      nas.local\t0\t(shortcut)\tstandard::display-name=NAS\tstandard::target-uri=smb://nas.local/\n\
                      printer\t0\t(shortcut)\tstandard::display-name=Printer\tstandard::target-uri=ipp://printer/\n\
                      box\t0\t(shortcut)\tstandard::target-uri=sftp://box.local/\n\
                      \n";
        assert_eq!(
            parse_gio_list(output),
            [
                NetworkHost {
                    name: "box".into(),
                    uri: "sftp://box.local/".into()
                },
                NetworkHost {
                    name: "NAS".into(),
                    uri: "smb://nas.local/".into()
                },
                NetworkHost {
                    name: "Windows Network".into(),
                    uri: "smb:///".into()
                },
            ]
        );
    }

    #[test]
    fn only_found_computers_are_network_addresses() {
        assert_eq!(
            network_address(Path::new("smb://nas.local/")).as_deref(),
            Some("smb://nas.local/")
        );
        assert_eq!(network_address(Path::new("/home/me/smb:")), None);
        assert_eq!(network_address(Path::new("notes.txt")), None);
        let entry = host_entry(&NetworkHost {
            name: "NAS".into(),
            uri: "smb://nas.local/".into(),
        });
        assert_eq!(network_address(&entry.path).as_deref(), Some("smb://nas.local/"));
        assert!(!entry.is_dir);
    }
}
