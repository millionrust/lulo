//! Users & Groups and Login Password: AccountsService on blocking workers.
//!
//! Secrets typed into the sheets are copied once into
//! `rmac_users_linux::Secret` (zeroized on drop), the fields are cleared at
//! the same moment, and nothing here logs or formats them. The caller's own
//! password changes through `passwd` (PAM checks the old one); a new
//! account's first password crosses D-Bus only as a SHA-512 crypt hash.

use super::*;
mod render;

use rmac_users_linux::model::{
    check_new_password, people, suggest_user_name, validate_full_name, validate_user_name,
    AccountType, Secret, User,
};
use rmac_users_linux::AccountsService;

/// Largest picture file Settings will decode for an account picture.
const MAX_PICTURE_BYTES: u64 = 20 * 1024 * 1024;
/// Account pictures are stored square at this size (AccountsService and the
/// login screen show them at most this large).
const PICTURE_SIZE: u32 = 256;
const SYSTEM_FACES: &str = "/usr/share/pixmaps/faces";

#[derive(Default)]
pub(super) struct UsersState {
    pub(super) list: Vec<User>,
    pub(super) current_uid: u64,
    pub(super) loading: bool,
    pub(super) loaded: bool,
    pub(super) busy: bool,
    pub(super) error: Option<SharedString>,
    pub(super) watch_started: bool,
    pub(super) info: Option<InfoSheet>,
    pub(super) new_user: Option<NewUserSheet>,
    pub(super) password: Option<PasswordSheet>,
    pub(super) picture: Option<PictureSheet>,
    pub(super) delete: Option<DeleteConfirm>,
}

impl UsersState {
    pub(super) fn current(&self) -> Option<&User> {
        self.list.iter().find(|user| user.uid == self.current_uid)
    }

    fn user(&self, path: &str) -> Option<&User> {
        self.list.iter().find(|user| user.path == path)
    }

    pub(super) fn current_is_admin(&self) -> bool {
        self.current()
            .is_some_and(|user| user.account_type == AccountType::Administrator)
    }
}

/// The ⓘ sheet of one user.
pub(super) struct InfoSheet {
    pub(super) path: String,
    pub(super) full_name: Entity<InputState>,
    pub(super) error: Option<SharedString>,
}

pub(super) struct NewUserSheet {
    pub(super) account_type: AccountType,
    pub(super) full_name: Entity<InputState>,
    pub(super) user_name: Entity<InputState>,
    pub(super) password: Entity<InputState>,
    pub(super) verify: Entity<InputState>,
    pub(super) hint: Entity<InputState>,
    /// The account name was typed by hand, so stop suggesting one.
    pub(super) user_name_edited: bool,
    pub(super) error: Option<SharedString>,
    _subscriptions: Vec<gpui::Subscription>,
}

pub(super) struct PasswordSheet {
    pub(super) old: Entity<InputState>,
    pub(super) new: Entity<InputState>,
    pub(super) verify: Entity<InputState>,
    pub(super) hint: Entity<InputState>,
    pub(super) error: Option<SharedString>,
}

pub(super) struct PictureSheet {
    pub(super) path: String,
    pub(super) choices: Vec<PathBuf>,
    pub(super) error: Option<SharedString>,
}

/// The Mac's delete-user alert: keep or delete the home folder.
pub(super) struct DeleteConfirm {
    pub(super) uid: u64,
    pub(super) name: String,
    pub(super) delete_home: bool,
}

fn field(
    window: &mut Window,
    cx: &mut Context<Settings>,
    placeholder: &'static str,
) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
}

fn secret_field(
    window: &mut Window,
    cx: &mut Context<Settings>,
    placeholder: &'static str,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .masked(true)
    })
}

/// Copy a password field into a `Secret` and clear the field.
fn take_secret(field: &Entity<InputState>, window: &mut Window, cx: &mut App) -> Secret {
    let secret = Secret::new(field.read(cx).value().to_string());
    field.update(cx, |state, cx| state.set_value("", window, cx));
    secret
}

fn clear(fields: &[&Entity<InputState>], window: &mut Window, cx: &mut App) {
    for field in fields {
        field.update(cx, |state, cx| state.set_value("", window, cx));
    }
}

fn read_users() -> Result<(Vec<User>, u64), rmac_users_linux::Error> {
    let uid = rmac_users_linux::accounts::current_uid();
    let users = AccountsService::system()?.users()?;
    Ok((people(users, uid), uid))
}

/// The pictures Linux ships for accounts (GNOME's faces), if installed.
fn system_faces() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(SYSTEM_FACES) else {
        return Vec::new();
    };
    let mut faces: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| matches!(ext, "png" | "jpg" | "jpeg"))
        })
        .collect();
    faces.sort();
    faces.truncate(24);
    faces
}

/// Decode a picture the user chose (bounded), crop it square from the
/// centre, scale it to 256 pt and write it as a private PNG for
/// AccountsService to copy. The caller removes the file afterwards.
fn prepare_picture(source: &std::path::Path) -> Result<PathBuf, &'static str> {
    const UNREADABLE: &str = "This picture can’t be used. Choose a PNG, JPEG or WebP image.";
    let metadata = std::fs::metadata(source).map_err(|_| UNREADABLE)?;
    if !metadata.is_file() || metadata.len() > MAX_PICTURE_BYTES {
        return Err("Choose a picture smaller than 20 MB.");
    }
    let reader = image::ImageReader::open(source)
        .and_then(|reader| reader.with_guessed_format())
        .map_err(|_| UNREADABLE)?;
    let mut reader = reader;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(12_000);
    limits.max_image_height = Some(12_000);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let picture = reader.decode().map_err(|_| UNREADABLE)?;
    let side = picture.width().min(picture.height());
    let left = (picture.width() - side) / 2;
    let top = (picture.height() - side) / 2;
    let square = picture.crop_imm(left, top, side, side).resize_exact(
        PICTURE_SIZE,
        PICTURE_SIZE,
        image::imageops::FilterType::Lanczos3,
    );
    let directory = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    let target = directory.join(format!("lulo-account-picture-{}.png", std::process::id()));
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&target)
            .map_err(|_| UNREADABLE)?;
        let mut writer = std::io::BufWriter::new(file);
        square
            .to_rgba8()
            .write_to(&mut writer, image::ImageFormat::Png)
            .map_err(|_| UNREADABLE)?;
    }
    Ok(target)
}

impl Settings {
    pub(super) fn refresh_users(&mut self, cx: &mut Context<Self>) {
        if !self.users.watch_started {
            self.users.watch_started = true;
            Self::watch_users(cx);
        }
        self.users.loading = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(read_users).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.users.loading = false;
                this.users.loaded = true;
                match result {
                    Ok((list, uid)) => {
                        this.users.list = list;
                        this.users.current_uid = uid;
                        this.users.error = None;
                    }
                    Err(error) => this.users.error = Some(error.message().into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Re-read whenever AccountsService announces a change (another app or
    /// user edited an account). Signal-driven; no polling.
    fn watch_users(cx: &mut Context<Self>) {
        let (sender, receiver) = async_channel::bounded(1);
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while receiver.recv().await.is_ok() {
                let _ = this.update(cx, |this: &mut Settings, cx| {
                    if !this.users.busy {
                        this.refresh_users(cx);
                    }
                });
            }
        })
        .detach();
        cx.spawn(async move |_, _| {
            blocking::unblock(move || {
                if let Ok(service) = AccountsService::system() {
                    let _ = service.watch(&mut |_| {
                        let _ = sender.try_send(());
                    });
                }
            })
            .await;
        })
        .detach();
    }

    /// Run one AccountsService mutation on a worker, then re-read.
    fn mutate_users(
        &mut self,
        work: impl FnOnce() -> Result<(), rmac_users_linux::Error> + Send + 'static,
        done: impl FnOnce(&mut Settings, Result<(), rmac_users_linux::Error>, &mut Context<Settings>)
            + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.users.busy {
            return;
        }
        self.users.busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(work).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.users.busy = false;
                done(this, result, cx);
                this.refresh_users(cx);
            });
        })
        .detach();
    }

    pub(super) fn open_user_info(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(user) = self.users.user(&path) else {
            return;
        };
        let name = user.real_name.clone();
        self.users.info = Some(InfoSheet {
            path,
            full_name: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Full Name")
                    .default_value(name)
            }),
            error: None,
        });
        cx.notify();
    }

    pub(super) fn close_user_sheets(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.users.busy {
            return;
        }
        if let Some(sheet) = self.users.new_user.take() {
            clear(&[&sheet.password, &sheet.verify], window, cx);
        }
        if let Some(sheet) = self.users.password.take() {
            clear(&[&sheet.old, &sheet.new, &sheet.verify], window, cx);
        }
        self.users.info = None;
        self.users.picture = None;
        self.users.delete = None;
        cx.notify();
    }

    /// OK in the ⓘ sheet: save the full name when it is yours and changed.
    pub(super) fn save_user_info(&mut self, cx: &mut Context<Self>) {
        let Some(sheet) = self.users.info.as_mut() else {
            return;
        };
        let name = sheet.full_name.read(cx).value().trim().to_owned();
        let path = sheet.path.clone();
        let Some(user) = self.users.user(&path) else {
            self.users.info = None;
            cx.notify();
            return;
        };
        if user.uid != self.users.current_uid || name == user.real_name {
            self.users.info = None;
            cx.notify();
            return;
        }
        if let Err(problem) = validate_full_name(&name) {
            if let Some(sheet) = self.users.info.as_mut() {
                sheet.error = Some(
                    if problem == rmac_users_linux::NameProblem::Characters {
                        "The full name can’t contain colons, commas or line breaks."
                    } else {
                        problem.message()
                    }
                    .into(),
                );
            }
            cx.notify();
            return;
        }
        self.mutate_users(
            move || AccountsService::system()?.set_real_name(&path, &name),
            |this, result, cx| {
                match result {
                    Ok(()) => this.users.info = None,
                    Err(error) => {
                        if let Some(sheet) = this.users.info.as_mut() {
                            sheet.error = Some(error.message().into());
                        }
                    }
                }
                cx.notify();
            },
            cx,
        );
    }

    pub(super) fn open_new_user(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let full_name = field(window, cx, "Full Name");
        let user_name = field(window, cx, "Account name");
        let subscriptions = vec![
            cx.subscribe_in(
                &full_name,
                window,
                |this, field, event: &InputEvent, window, cx| {
                    if !matches!(event, InputEvent::Change) {
                        return;
                    }
                    let suggestion = suggest_user_name(&field.read(cx).value());
                    if let Some(sheet) = this
                        .users
                        .new_user
                        .as_ref()
                        .filter(|sheet| !sheet.user_name_edited)
                    {
                        let user_name = sheet.user_name.clone();
                        user_name.update(cx, |state, cx| state.set_value(suggestion, window, cx));
                    }
                },
            ),
            cx.subscribe_in(
                &user_name,
                window,
                |this, field, event: &InputEvent, _, cx| {
                    if !matches!(event, InputEvent::Change) {
                        return;
                    }
                    let typed = field.read(cx).value().to_string();
                    let full = this
                        .users
                        .new_user
                        .as_ref()
                        .map(|sheet| suggest_user_name(&sheet.full_name.read(cx).value()));
                    if let Some(sheet) = this.users.new_user.as_mut() {
                        if full.as_deref() != Some(typed.as_str()) {
                            sheet.user_name_edited = true;
                        }
                    }
                },
            ),
        ];
        self.users.new_user = Some(NewUserSheet {
            account_type: AccountType::Standard,
            full_name,
            user_name,
            password: secret_field(window, cx, "Required"),
            verify: secret_field(window, cx, "Verify"),
            hint: field(window, cx, "Recommended"),
            user_name_edited: false,
            error: None,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    pub(super) fn submit_new_user(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = self.users.new_user.as_mut() else {
            return;
        };
        let full_name = sheet.full_name.read(cx).value().trim().to_owned();
        let user_name = sheet.user_name.read(cx).value().trim().to_owned();
        let hint = sheet.hint.read(cx).value().trim().to_owned();
        let account_type = sheet.account_type;
        let problem = validate_full_name(&full_name)
            .err()
            .map(|problem| match problem {
                rmac_users_linux::NameProblem::Characters => {
                    "The full name can’t contain colons, commas or line breaks."
                }
                other => other.message(),
            })
            .or_else(|| {
                validate_user_name(&user_name)
                    .err()
                    .map(|problem| problem.message())
            })
            .or_else(|| {
                self.users
                    .list
                    .iter()
                    .any(|user| user.user_name == user_name)
                    .then_some(rmac_users_linux::Error::UserExists.message())
            });
        let sheet = self.users.new_user.as_mut().expect("sheet checked above");
        if let Some(problem) = problem {
            sheet.error = Some(problem.into());
            cx.notify();
            return;
        }
        let password = take_secret(&sheet.password, window, cx);
        let verify = take_secret(&sheet.verify, window, cx);
        if let Err(problem) = check_new_password(&password, &verify, &hint) {
            sheet.error = Some(problem.message().into());
            cx.notify();
            return;
        }
        drop(verify);
        self.mutate_users(
            move || {
                let service = AccountsService::system()?;
                let hash = rmac_users_linux::crypt::hash_password(password.expose())?;
                drop(password);
                let path = service.create_user(&user_name, &full_name, account_type)?;
                if let Err(error) = service.set_password_hash(&path, &hash, &hint) {
                    // Never leave an account behind whose password the
                    // administrator did not get to set.
                    if let Ok(user) = service.user(&path) {
                        let _ = service.delete_user(user.uid, true);
                    }
                    return Err(error);
                }
                Ok(())
            },
            |this, result, cx| {
                match result {
                    Ok(()) => this.users.new_user = None,
                    Err(error) => {
                        if let Some(sheet) = this.users.new_user.as_mut() {
                            sheet.error = Some(error.message().into());
                        }
                    }
                }
                cx.notify();
            },
            cx,
        );
    }

    pub(super) fn open_change_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.users.info = None;
        let hint = self
            .users
            .current()
            .map(|user| user.password_hint.clone())
            .unwrap_or_default();
        self.users.password = Some(PasswordSheet {
            old: secret_field(window, cx, "Required"),
            new: secret_field(window, cx, "Required"),
            verify: secret_field(window, cx, "Verify"),
            hint: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Recommended")
                    .default_value(hint)
            }),
            error: None,
        });
        cx.notify();
    }

    pub(super) fn submit_change_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.users.busy {
            return;
        }
        let path = self.users.current().map(|user| user.path.clone());
        let old_hint = self
            .users
            .current()
            .map(|user| user.password_hint.clone())
            .unwrap_or_default();
        let Some(sheet) = self.users.password.as_mut() else {
            return;
        };
        let hint = sheet.hint.read(cx).value().trim().to_owned();
        let old = take_secret(&sheet.old, window, cx);
        let new = take_secret(&sheet.new, window, cx);
        let verify = take_secret(&sheet.verify, window, cx);
        if old.is_empty() {
            sheet.error = Some("Enter your old password.".into());
            cx.notify();
            return;
        }
        if let Err(problem) = check_new_password(&new, &verify, &hint) {
            sheet.error = Some(problem.message().into());
            cx.notify();
            return;
        }
        drop(verify);
        self.users.busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                #[cfg(target_os = "linux")]
                let changed = rmac_users_linux::passwd::change_own_password(&old, &new)
                    .map_err(|error| error.message());
                #[cfg(not(target_os = "linux"))]
                let changed: Result<(), String> = {
                    let _ = (&old, &new);
                    Err(rmac_users_linux::passwd::PasswdError::Unavailable.message())
                };
                drop((old, new));
                changed?;
                if hint != old_hint {
                    if let Some(path) = path {
                        // The password changed; a hint failure is reported
                        // but does not undo it.
                        AccountsService::system()
                            .and_then(|service| service.set_password_hint(&path, &hint))
                            .map_err(|_| {
                                "Your password was changed, but the password hint couldn’t be saved."
                                    .to_owned()
                            })?;
                    }
                }
                Ok::<(), String>(())
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.users.busy = false;
                match result {
                    Ok(()) => this.users.password = None,
                    Err(message) => {
                        if let Some(sheet) = this.users.password.as_mut() {
                            sheet.error = Some(message.into());
                        }
                    }
                }
                this.refresh_users(cx);
            });
        })
        .detach();
    }

    pub(super) fn open_picture_sheet(&mut self, path: String, cx: &mut Context<Self>) {
        self.users.info = None;
        self.users.picture = Some(PictureSheet {
            path,
            choices: Vec::new(),
            error: None,
        });
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let faces = blocking::unblock(system_faces).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if let Some(sheet) = this.users.picture.as_mut() {
                    sheet.choices = faces;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Use one of the system's account pictures.
    pub(super) fn set_user_picture(&mut self, picture: PathBuf, cx: &mut Context<Self>) {
        let Some(path) = self.users.picture.as_ref().map(|sheet| sheet.path.clone()) else {
            return;
        };
        self.mutate_users(
            move || AccountsService::system()?.set_icon_file(&path, &picture),
            Self::picture_done,
            cx,
        );
    }

    fn picture_done(
        this: &mut Settings,
        result: Result<(), rmac_users_linux::Error>,
        cx: &mut Context<Settings>,
    ) {
        match result {
            Ok(()) => this.users.picture = None,
            Err(error) => {
                if let Some(sheet) = this.users.picture.as_mut() {
                    sheet.error = Some(error.message().into());
                }
            }
        }
        cx.notify();
    }

    /// Choose a picture from Files, then crop, scale and store it.
    pub(super) fn choose_user_picture_file(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.users.picture.as_ref().map(|sheet| sheet.path.clone()) else {
            return;
        };
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let chosen = rmac_portal::choose_user_picture().await;
            let _ = this.update(cx, |this: &mut Settings, cx| match chosen {
                Ok(Some(file)) => {
                    let prepared = std::sync::Arc::new(std::sync::Mutex::new(None::<&'static str>));
                    let failure = prepared.clone();
                    this.mutate_users(
                        move || {
                            let picture = match prepare_picture(&file) {
                                Ok(picture) => picture,
                                Err(message) => {
                                    *failure.lock().unwrap() = Some(message);
                                    return Err(rmac_users_linux::Error::Failed);
                                }
                            };
                            let result = AccountsService::system()
                                .and_then(|service| service.set_icon_file(&path, &picture));
                            let _ = std::fs::remove_file(&picture);
                            result
                        },
                        move |this, result, cx| {
                            let message = prepared.lock().unwrap().take();
                            Self::picture_done(this, result, cx);
                            if let (Some(message), Some(sheet)) =
                                (message, this.users.picture.as_mut())
                            {
                                sheet.error = Some(message.into());
                            }
                        },
                        cx,
                    );
                }
                Ok(None) => {}
                Err(_) => {
                    if let Some(sheet) = this.users.picture.as_mut() {
                        sheet.error = Some("The file chooser couldn’t open.".into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    pub(super) fn request_delete_user(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(user) = self.users.user(path) else {
            return;
        };
        if user.uid == self.users.current_uid {
            return;
        }
        self.users.delete = Some(DeleteConfirm {
            uid: user.uid,
            name: user.display_name().to_owned(),
            delete_home: false,
        });
        self.users.info = None;
        cx.notify();
    }

    pub(super) fn confirm_delete_user(&mut self, cx: &mut Context<Self>) {
        let Some(confirm) = self.users.delete.as_ref() else {
            return;
        };
        let (uid, delete_home) = (confirm.uid, confirm.delete_home);
        self.mutate_users(
            move || AccountsService::system()?.delete_user(uid, delete_home),
            |this, result, cx| {
                this.users.delete = None;
                if let Err(error) = result {
                    this.users.error = Some(error.message().into());
                }
                cx.notify();
            },
            cx,
        );
    }

    /// Login Options ▸ Automatically log in as: `None` turns it off.
    pub(super) fn set_automatic_login(&mut self, target: Option<String>, cx: &mut Context<Self>) {
        let current: Option<String> = self
            .users
            .list
            .iter()
            .find(|user| user.automatic_login)
            .map(|user| user.path.clone());
        if current == target {
            return;
        }
        self.mutate_users(
            move || {
                let service = AccountsService::system()?;
                match target {
                    Some(path) => service.set_automatic_login(&path, true),
                    None => match current {
                        Some(path) => service.set_automatic_login(&path, false),
                        None => Ok(()),
                    },
                }
            },
            |this, result, cx| {
                if let Err(error) = result {
                    this.users.error = Some(error.message().into());
                }
                cx.notify();
            },
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chosen_pictures_are_cropped_square_and_scaled() {
        let dir =
            std::env::temp_dir().join(format!("rmac-settings-picture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("wide.png");
        image::RgbaImage::from_pixel(600, 300, image::Rgba([10, 20, 30, 255]))
            .save(&source)
            .unwrap();
        let prepared = prepare_picture(&source).unwrap();
        let decoded = image::open(&prepared).unwrap();
        assert_eq!(
            (decoded.width(), decoded.height()),
            (PICTURE_SIZE, PICTURE_SIZE)
        );
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&prepared).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_file(prepared);
        std::fs::write(dir.join("not-a-picture.png"), b"hello").unwrap();
        assert!(prepare_picture(&dir.join("not-a-picture.png")).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
