//! Mail ▸ Settings… (⌘,) domain model (MAIL-8): General, Junk Mail, Fonts &
//! Colours, Viewing, Composing, Signatures and Privacy. Accounts has no
//! state of its own here — it reads live accounts from `rmac-accounts`
//! (MAIL-4's runtime boundary) and shows server settings read-only for
//! OAuth accounts, per `docs/design/calendar-mail.md` §3.
//!
//! Persisted as one JSON document at `~/.config/lulo/mail-settings.json`
//! (ADR 0022 §9's `~/.config/lulo/` convention for small UI-state files),
//! directory 0700, file 0600, atomic writes — mirroring
//! `crates/terminal/src/settings.rs`.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use rmac_mail_mime::Draft;

const VERSION: u32 = 1;
const FILE_NAME: &str = "mail-settings.json";
const MAX_FILE_BYTES: usize = 256 * 1024;

/// Mail ▸ Settings… ▸ Junk Mail's local-filter choice. The server Junk
/// mailbox (IMAP SPECIAL-USE) is always honoured regardless of this value.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum JunkMailAction {
    #[default]
    LeaveInInbox,
    MoveToJunk,
}

impl JunkMailAction {
    pub const ALL: [Self; 2] = [Self::LeaveInInbox, Self::MoveToJunk];

    pub fn label(self) -> &'static str {
        match self {
            Self::LeaveInInbox => "Mark as Junk Mail but leave it in my Inbox",
            Self::MoveToJunk => "Move it to the Junk mailbox",
        }
    }
}

/// Mail ▸ Settings… ▸ Composing ▸ "Message Format".
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ComposeFormat {
    #[default]
    RichText,
    PlainText,
}

impl ComposeFormat {
    pub const ALL: [Self; 2] = [Self::RichText, Self::PlainText];

    pub fn label(self) -> &'static str {
        match self {
            Self::RichText => "Rich Text",
            Self::PlainText => "Plain Text",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct GeneralSettings {
    pub notify_new_mail: bool,
    pub play_sound_for_new_mail: bool,
    pub downloads_folder: String,
    pub remove_unedited_downloads: bool,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            notify_new_mail: true,
            play_sound_for_new_mail: true,
            downloads_folder: "Downloads".to_owned(),
            remove_unedited_downloads: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct JunkMailSettings {
    pub filter_enabled: bool,
    pub action: JunkMailAction,
}

impl Default for JunkMailSettings {
    fn default() -> Self {
        Self {
            filter_enabled: true,
            action: JunkMailAction::default(),
        }
    }
}

/// Mail ▸ Settings… ▸ Fonts & Colours.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct FontsAndColoursSettings {
    pub message_list_font_size: u8,
    pub message_font_size: u8,
    pub fixed_width_for_plain_text: bool,
}

impl Default for FontsAndColoursSettings {
    fn default() -> Self {
        Self {
            message_list_font_size: 13,
            message_font_size: 13,
            fixed_width_for_plain_text: false,
        }
    }
}

/// Mail ▸ Settings… ▸ Viewing.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct ViewingSettings {
    pub preview_lines: u8,
    pub show_to_cc_in_list: bool,
    pub load_remote_content: bool,
}

impl Default for ViewingSettings {
    fn default() -> Self {
        Self {
            preview_lines: 2,
            show_to_cc_in_list: false,
            load_remote_content: false,
        }
    }
}

/// Mail ▸ Settings… ▸ Composing.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct ComposingSettings {
    pub format: ComposeFormat,
    pub quote_original_when_replying: bool,
    pub check_spelling_while_typing: bool,
}

impl Default for ComposingSettings {
    fn default() -> Self {
        Self {
            format: ComposeFormat::default(),
            quote_original_when_replying: true,
            check_spelling_while_typing: true,
        }
    }
}

/// Mail ▸ Settings… ▸ Privacy. Separate from `ViewingSettings::load_remote_content`,
/// which is Lulo's actual per-message default; this mirrors the Mac's
/// dedicated "Block All Remote Content" switch, which some Mac users expect
/// to find in its own place.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct PrivacySettings {
    pub block_all_remote_content: bool,
}

/// A rich-text-by-convention signature body (plain text today; MAIL-8 keeps
/// the body a plain string, matching `Draft::text`, until a rich-text editor
/// control exists in `rmac-ui`). Chosen automatically per account.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Signature {
    pub id: String,
    pub name: String,
    pub body: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct SignatureSettings {
    pub signatures: Vec<Signature>,
    /// Account address → signature id. An account with no entry here gets
    /// no signature, matching the Mac's "None" default for a new account.
    pub default_for_account: std::collections::BTreeMap<String, String>,
}

impl SignatureSettings {
    pub fn signature(&self, id: &str) -> Option<&Signature> {
        self.signatures.iter().find(|signature| signature.id == id)
    }

    /// The signature automatically chosen for `account_address`, if any.
    pub fn signature_for_account(&self, account_address: &str) -> Option<&Signature> {
        let id = self.default_for_account.get(account_address)?;
        self.signature(id)
    }

    /// A free slug for a new signature, distinct from every existing id.
    pub fn next_id(&self) -> String {
        let mut index = self.signatures.len() + 1;
        loop {
            let candidate = format!("sig-{index}");
            if self.signature(&candidate).is_none() {
                return candidate;
            }
            index += 1;
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct MailSettings {
    pub general: GeneralSettings,
    pub junk: JunkMailSettings,
    pub fonts_and_colours: FontsAndColoursSettings,
    pub viewing: ViewingSettings,
    pub composing: ComposingSettings,
    pub signatures: SignatureSettings,
    pub privacy: PrivacySettings,
}

impl MailSettings {
    /// Appends `account_address`'s automatic signature to `draft`'s plain
    /// text body, "-- " below the body as `design-lab/mail.html`'s compose
    /// mock shows. A draft with no matching signature is returned unchanged
    /// — this is the hook MAIL-6's compose window calls on every new
    /// message, reply and forward once an account is chosen.
    pub fn compose_draft_with_signature(&self, mut draft: Draft, account_address: &str) -> Draft {
        if let Some(signature) = self.signatures.signature_for_account(account_address) {
            if !draft.text.is_empty() {
                draft.text.push_str("\n\n");
            }
            draft.text.push_str("--\n");
            draft.text.push_str(&signature.body);
        }
        draft
    }
}

#[derive(Deserialize, Serialize)]
struct StoredSettings {
    version: u32,
    settings: MailSettings,
}

fn settings_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let config = match std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
        Some(path) if path.is_absolute() => path,
        _ => home.join(".config"),
    };
    config.join("lulo").join(FILE_NAME)
}

fn load_from(path: &Path) -> io::Result<MailSettings> {
    let bytes = match rmac_storage::read_bounded_no_follow(path, MAX_FILE_BYTES) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(MailSettings::default()),
        Err(error) => return Err(error),
    };
    let stored: StoredSettings = serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if stored.version != VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "mail-settings.json version {} is not supported",
                stored.version
            ),
        ));
    }
    Ok(stored.settings)
}

fn save_to(path: &Path, settings: &MailSettings) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        rmac_storage::create_dir_all_private(parent)?;
    }
    let document = StoredSettings {
        version: VERSION,
        settings: settings.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&document)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    rmac_storage::atomic_write_private(path, &bytes)
}

/// Loads the saved Settings, or the Mac-matching defaults if none were ever
/// saved or the file is damaged — the Settings window must still open. The
/// error, when there was one, is for the window to show as a banner.
pub fn load() -> (MailSettings, Option<String>) {
    match load_from(&settings_path()) {
        Ok(settings) => (settings, None),
        Err(error) => (MailSettings::default(), Some(error.to_string())),
    }
}

pub fn save(settings: &MailSettings) -> io::Result<()> {
    save_to(&settings_path(), settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work_signature() -> Signature {
        Signature {
            id: "sig-work".to_owned(),
            name: "Work".to_owned(),
            body: "Jacob Samas\nLulo OS".to_owned(),
        }
    }

    #[test]
    fn defaults_match_the_design_doc() {
        let settings = MailSettings::default();
        assert!(settings.general.notify_new_mail);
        assert!(settings.general.play_sound_for_new_mail);
        assert_eq!(settings.general.downloads_folder, "Downloads");
        assert!(settings.junk.filter_enabled);
        assert_eq!(settings.junk.action, JunkMailAction::LeaveInInbox);
        assert_eq!(settings.viewing.preview_lines, 2);
        assert!(!settings.viewing.load_remote_content);
        assert!(!settings.privacy.block_all_remote_content);
        assert_eq!(settings.composing.format, ComposeFormat::RichText);
        assert!(settings.signatures.signatures.is_empty());
    }

    #[test]
    fn signature_selection_is_per_account_and_absent_by_default() {
        let mut settings = MailSettings::default();
        settings.signatures.signatures.push(work_signature());
        assert!(settings
            .signatures
            .signature_for_account("jacob@example.com")
            .is_none());
        settings
            .signatures
            .default_for_account
            .insert("jacob@example.com".to_owned(), "sig-work".to_owned());
        assert_eq!(
            settings
                .signatures
                .signature_for_account("jacob@example.com")
                .unwrap()
                .name,
            "Work"
        );
        assert!(settings
            .signatures
            .signature_for_account("other@example.com")
            .is_none());
    }

    #[test]
    fn next_id_skips_existing_signatures() {
        let mut settings = SignatureSettings::default();
        settings.signatures.push(work_signature());
        let next = settings.next_id();
        assert!(settings.signature(&next).is_none());
    }

    #[test]
    fn compose_draft_with_signature_appends_below_a_dash_dash_line() {
        let mut settings = MailSettings::default();
        settings.signatures.signatures.push(work_signature());
        settings
            .signatures
            .default_for_account
            .insert("jacob@example.com".to_owned(), "sig-work".to_owned());
        let draft = Draft {
            text: "Perfect, see you there.".to_owned(),
            ..Draft::default()
        };
        let signed = settings.compose_draft_with_signature(draft, "jacob@example.com");
        assert_eq!(
            signed.text,
            "Perfect, see you there.\n\n--\nJacob Samas\nLulo OS"
        );
    }

    #[test]
    fn compose_draft_without_a_matching_signature_is_unchanged() {
        let settings = MailSettings::default();
        let draft = Draft {
            text: "Hi".to_owned(),
            ..Draft::default()
        };
        let unsigned = settings.compose_draft_with_signature(draft, "nobody@example.com");
        assert_eq!(unsigned.text, "Hi");
    }

    #[test]
    fn empty_body_signature_has_no_leading_blank_line() {
        let mut settings = MailSettings::default();
        settings.signatures.signatures.push(work_signature());
        settings
            .signatures
            .default_for_account
            .insert("jacob@example.com".to_owned(), "sig-work".to_owned());
        let signed = settings.compose_draft_with_signature(Draft::default(), "jacob@example.com");
        assert_eq!(signed.text, "--\nJacob Samas\nLulo OS");
    }

    #[test]
    fn missing_settings_file_falls_back_to_defaults_without_an_error() {
        let directory = std::env::temp_dir().join(format!(
            "rmac-mail-settings-test-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join(FILE_NAME);
        assert_eq!(load_from(&path).unwrap(), MailSettings::default());
    }

    #[test]
    fn round_trips_through_json_on_disk() {
        let directory = std::env::temp_dir().join(format!(
            "rmac-mail-settings-test-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join(FILE_NAME);
        let mut settings = MailSettings::default();
        settings.signatures.signatures.push(work_signature());
        settings.junk.action = JunkMailAction::MoveToJunk;
        save_to(&path, &settings).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded, settings);
        let mode = std::fs::metadata(&path).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(mode.mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_damaged_future_version_is_reported_not_silently_reset() {
        let directory = std::env::temp_dir().join(format!(
            "rmac-mail-settings-test-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join(FILE_NAME);
        std::fs::write(&path, br#"{"version":999,"settings":{}}"#).unwrap();
        assert!(load_from(&path).is_err());
        let (settings, error) = {
            // load()/save() use the real $HOME; exercise the same fallback
            // behaviour load() relies on through load_from() directly.
            match load_from(&path) {
                Ok(settings) => (settings, None),
                Err(error) => (MailSettings::default(), Some(error.to_string())),
            }
        };
        assert_eq!(settings, MailSettings::default());
        assert!(error.is_some());
        let _ = std::fs::remove_dir_all(&directory);
    }
}
