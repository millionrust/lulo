//! Finder's search scope bar (FILES-37): "Search: This Mac | “<folder>”",
//! and the + criteria row that File ▸ Find (⌘F) opens with "Kind is Any".
//!
//! The scope starts where Finder ▸ Settings ▸ Advanced ▸ "When performing a
//! search" says. "This Mac" searches the home folder tree, where the
//! person's own files live; the folder scope searches the open folder tree.
//! Changing the scope or the kind while results are shown searches again.

use super::settings::SearchScope;
use super::*;
use rmac_ui::{PopUpButton, PopupMenuItem};

const SCOPE_BAR_HEIGHT: f32 = 32.0;

/// The kinds Finder's "Kind is" criterion offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SearchKind {
    Any,
    Application,
    Archive,
    Document,
    Folder,
    Image,
    Movie,
    Music,
    Pdf,
    Presentation,
    Text,
}

impl SearchKind {
    pub(super) const ALL: [SearchKind; 11] = [
        Self::Any,
        Self::Application,
        Self::Archive,
        Self::Document,
        Self::Folder,
        Self::Image,
        Self::Movie,
        Self::Music,
        Self::Pdf,
        Self::Presentation,
        Self::Text,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Any => "Any",
            Self::Application => "Application",
            Self::Archive => "Archive",
            Self::Document => "Document",
            Self::Folder => "Folder",
            Self::Image => "Image",
            Self::Movie => "Movie",
            Self::Music => "Music",
            Self::Pdf => "PDF",
            Self::Presentation => "Presentation",
            Self::Text => "Text",
        }
    }

    /// Whether an item of this name matches, judged as Finder's Spotlight
    /// kinds are: by folder-ness and by the type its extension names.
    pub(super) fn matches(self, path: &Path, is_dir: bool, application: bool) -> bool {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let ext = extension.as_str();
        const TEXT: &[&str] = &[
            "txt", "md", "markdown", "rtf", "csv", "tsv", "log", "json", "xml", "yaml", "yml",
            "toml", "ini", "conf", "html", "htm", "css", "js", "ts", "py", "rs", "c", "h", "cpp",
            "hpp", "java", "sh", "go", "rb", "swift", "kt", "sql",
        ];
        const IMAGE: &[&str] = &[
            "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "heic", "heif", "svg",
            "ico", "avif", "raw",
        ];
        const MOVIE: &[&str] = &["mp4", "m4v", "mov", "mkv", "webm", "avi", "mpg", "mpeg", "ogv"];
        const MUSIC: &[&str] = &["mp3", "m4a", "aac", "flac", "wav", "ogg", "oga", "opus", "aiff"];
        const ARCHIVE: &[&str] = &["zip", "tar", "gz", "tgz", "bz2", "xz", "zst", "7z", "rar"];
        const PRESENTATION: &[&str] = &["key", "ppt", "pptx", "odp"];
        const OFFICE: &[&str] = &[
            "pdf", "doc", "docx", "odt", "pages", "xls", "xlsx", "ods", "numbers", "epub",
        ];
        match self {
            Self::Any => true,
            Self::Folder => is_dir,
            _ if is_dir => false,
            Self::Application => {
                application || matches!(ext, "desktop" | "appimage" | "app" | "exe" | "msi")
            }
            Self::Archive => ARCHIVE.contains(&ext),
            Self::Image => IMAGE.contains(&ext),
            Self::Movie => MOVIE.contains(&ext),
            Self::Music => MUSIC.contains(&ext),
            Self::Pdf => ext == "pdf",
            Self::Presentation => PRESENTATION.contains(&ext),
            Self::Text => TEXT.contains(&ext),
            Self::Document => {
                TEXT.contains(&ext) || OFFICE.contains(&ext) || PRESENTATION.contains(&ext)
            }
        }
    }
}

/// Where a new search starts, from the Advanced setting and the last scope.
pub(super) fn starting_scope(setting: SearchScope, previous_this_mac: bool) -> bool {
    match setting {
        SearchScope::ThisMac => true,
        SearchScope::CurrentFolder => false,
        SearchScope::PreviousScope => previous_this_mac,
    }
}

/// The folder a search walks.
pub(super) fn search_root(this_mac: bool, home: &Path, folder: &Path) -> PathBuf {
    if this_mac {
        home.to_path_buf()
    } else {
        folder.to_path_buf()
    }
}

impl FinderView {
    /// Start a search the way the Advanced setting says, keeping the
    /// previous scope for "Use the Previous Search Scope".
    pub(super) fn reset_search_scope(&mut self) {
        let setting = super::settings::current().advanced.when_performing_search;
        self.search_this_mac = starting_scope(setting, self.search_this_mac);
    }

    /// File ▸ Find (⌘F) and Find by Name (⌃⇧⌘F): the search field with its
    /// scope bar shown at once; ⌘F also adds the "Kind is Any" criterion.
    pub(super) fn open_find(&mut self, name_only: bool, window: &mut Window, cx: &mut Context<Self>) {
        if name_only {
            self.open_name_search(window, cx);
        } else {
            self.open_search(window, cx);
        }
        self.search_find_mode = true;
        self.search_kind = (!name_only).then_some(SearchKind::Any);
        cx.notify();
    }

    pub(super) fn search_scope_visible(&self, cx: &gpui::App) -> bool {
        !self.trash_view
            && !self.applications_view
            && (self.search_find_mode
                || self.search_summary.is_some()
                || (self.search_open && !self.query.read(cx).value().is_empty()))
    }

    pub(super) fn set_search_scope(&mut self, this_mac: bool, cx: &mut Context<Self>) {
        if self.search_this_mac == this_mac {
            return;
        }
        self.search_this_mac = this_mac;
        self.search_again(cx);
    }

    pub(super) fn set_search_kind(&mut self, kind: Option<SearchKind>, cx: &mut Context<Self>) {
        if self.search_kind == kind {
            return;
        }
        self.search_kind = kind;
        self.search_again(cx);
    }

    fn search_again(&mut self, cx: &mut Context<Self>) {
        if self.search_summary.is_some() && !self.query.read(cx).value().trim().is_empty() {
            self.recursive_search(cx);
        } else {
            cx.notify();
        }
    }

    /// The name the folder scope shows: the open folder's own name.
    fn search_folder_label(&self) -> String {
        let name = self
            .cwd
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| rmac_finder::places::root_volume_name().to_owned());
        format!("“{}”", sanitize_dialog_name(&name))
    }

    fn scope_choice(
        &self,
        id: &'static str,
        text: String,
        selected: bool,
        this_mac: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .role(Role::RadioButton)
            .aria_label(text.clone())
            .aria_toggled(if selected {
                Toggled::True
            } else {
                Toggled::False
            })
            .h(px(22.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .rounded(px(rmac_ui::mac::radius_control()))
            .when(selected, |choice| choice.bg(stripe()).font_weight(rmac_ui::mac::SEMIBOLD))
            .text_color(primary_text())
            .cursor_pointer()
            .child(text)
            .on_click(cx.listener(move |this, _, _, cx| this.set_search_scope(this_mac, cx)))
    }

    pub(super) fn render_search_scope_bar(&self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.search_scope_visible(cx) {
            return None;
        }
        let entity = cx.entity().downgrade();
        let scope = div()
            .id("search-scope")
            .role(Role::RadioGroup)
            .aria_label("Search")
            .h(px(SCOPE_BAR_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.0))
            .px(px(6.0))
            .text_size(rmac_ui::text_px(12.0))
            .child(div().text_color(secondary_text()).child("Search:"))
            .child(self.scope_choice(
                "search-scope-this-mac",
                "This Mac".to_owned(),
                self.search_this_mac,
                true,
                cx,
            ))
            .child(self.scope_choice(
                "search-scope-folder",
                self.search_folder_label(),
                !self.search_this_mac,
                false,
                cx,
            ))
            .child(div().flex_1())
            .child(
                Button::new("search-add-criterion", "+")
                    .small()
                    .disabled(self.search_kind.is_some())
                    .tooltip("Add a search criterion")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_search_kind(Some(SearchKind::Any), cx)
                    })),
            );
        let criterion = self.search_kind.map(|kind| {
            let entity = entity.clone();
            div()
                .id("search-criterion")
                .role(Role::Group)
                .aria_label("Search criterion")
                .h(px(SCOPE_BAR_HEIGHT))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(6.0))
                .px(px(6.0))
                .border_t_1()
                .border_color(hairline())
                .text_size(rmac_ui::text_px(12.0))
                .child(div().text_color(primary_text()).child("Kind"))
                .child(div().text_color(secondary_text()).child("is"))
                .child(
                    PopUpButton::new("search-kind", kind.label())
                        .accessible_name("Kind")
                        .w(px(130.0))
                        .dropdown_menu(move |menu, _, _| {
                            SearchKind::ALL.iter().fold(menu, |menu, &choice| {
                                let entity = entity.clone();
                                menu.item(
                                    PopupMenuItem::new(choice.label())
                                        .checked(choice == kind)
                                        .on_click(move |_, _, cx| {
                                            let _ = entity.update(cx, |this: &mut FinderView, cx| {
                                                this.set_search_kind(Some(choice), cx)
                                            });
                                        }),
                                )
                            })
                        }),
                )
                .child(div().flex_1())
                .child(
                    Button::new("search-remove-criterion", "−")
                        .small()
                        .tooltip("Remove this search criterion")
                        .on_click(cx.listener(|this, _, _, cx| this.set_search_kind(None, cx))),
                )
        });
        Some(
            div()
                .v_flex()
                .flex_none()
                .border_b_1()
                .border_color(hairline())
                .child(scope)
                .children(criterion),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scope_follows_the_advanced_setting() {
        assert!(starting_scope(SearchScope::ThisMac, false));
        assert!(!starting_scope(SearchScope::CurrentFolder, true));
        assert!(starting_scope(SearchScope::PreviousScope, true));
        assert!(!starting_scope(SearchScope::PreviousScope, false));
        let home = Path::new("/home/me");
        let folder = Path::new("/home/me/Documents");
        assert_eq!(search_root(true, home, folder), home);
        assert_eq!(search_root(false, home, folder), folder);
    }

    #[test]
    fn kinds_match_like_finder_criteria() {
        let file = |name: &str| PathBuf::from(format!("/x/{name}"));
        assert!(SearchKind::Any.matches(&file("a"), true, false));
        assert!(SearchKind::Folder.matches(&file("a"), true, false));
        assert!(!SearchKind::Folder.matches(&file("a.txt"), false, false));
        assert!(!SearchKind::Image.matches(&file("a.png"), true, false));
        assert!(SearchKind::Image.matches(&file("Photo.JPG"), false, false));
        assert!(SearchKind::Pdf.matches(&file("a.pdf"), false, false));
        assert!(SearchKind::Document.matches(&file("a.pdf"), false, false));
        assert!(SearchKind::Document.matches(&file("notes.md"), false, false));
        assert!(SearchKind::Text.matches(&file("notes.md"), false, false));
        assert!(!SearchKind::Text.matches(&file("a.pdf"), false, false));
        assert!(SearchKind::Movie.matches(&file("clip.mkv"), false, false));
        assert!(SearchKind::Music.matches(&file("song.flac"), false, false));
        assert!(SearchKind::Archive.matches(&file("a.tar"), false, false));
        assert!(SearchKind::Presentation.matches(&file("deck.pptx"), false, false));
        assert!(SearchKind::Application.matches(&file("x.desktop"), false, false));
        assert!(SearchKind::Application.matches(&file("x"), false, true));
        assert!(!SearchKind::Application.matches(&file("x.txt"), false, false));
    }
}
