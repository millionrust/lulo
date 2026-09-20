use std::collections::BTreeSet;
#[cfg(target_os = "macos")]
use std::path::Path;
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::Command;

use gpui::SharedString;
use rmac_app_drawer::accessibility::{ApplicationCategory, ApplicationSemantics};

#[cfg(not(target_os = "macos"))]
use category::categorize_desktop;
#[cfg(target_os = "macos")]
use category::parallel_categorize;

/// App-Library-style buckets projected from platform catalog metadata.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Category {
    Productivity,
    Internet,
    Media,
    Developer,
    Utilities,
    Games,
    System,
    Other,
}

impl Category {
    pub(crate) fn label(self) -> &'static str {
        ApplicationCategory::from(self).label()
    }

    pub(crate) const ORDER: [Self; 8] = [
        Self::Productivity,
        Self::Internet,
        Self::Media,
        Self::Developer,
        Self::Utilities,
        Self::Games,
        Self::System,
        Self::Other,
    ];
}

impl From<Category> for ApplicationCategory {
    fn from(category: Category) -> Self {
        match category {
            Category::Productivity => Self::Productivity,
            Category::Internet => Self::Internet,
            Category::Media => Self::Media,
            Category::Developer => Self::Developer,
            Category::Utilities => Self::Utilities,
            Category::Games => Self::Games,
            Category::System => Self::System,
            Category::Other => Self::Other,
        }
    }
}

/// Renderer-ready application projection retaining the exact parsed launch
/// specifications and actions supplied by `rmac-apps`.
#[derive(Clone)]
pub(crate) struct App {
    pub(crate) id: String,
    pub(crate) first_party: bool,
    pub(crate) name: SharedString,
    pub(crate) generic_name: Option<String>,
    pub(crate) keywords: Vec<String>,
    pub(crate) path: PathBuf,
    pub(crate) icon: Option<PathBuf>,
    pub(crate) category: Category,
    pub(crate) source_categories: Vec<String>,
    pub(crate) mime_types: Vec<String>,
    pub(crate) search_text: String,
    pub(crate) launch: rmac_apps::LaunchSpec,
    pub(crate) actions: Vec<rmac_apps::DesktopAction>,
}

impl ApplicationSemantics for App {
    fn stable_id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        self.name.as_ref()
    }

    fn generic_name(&self) -> Option<&str> {
        self.generic_name.as_deref()
    }

    fn category(&self) -> ApplicationCategory {
        self.category.into()
    }

    fn declared_action_count(&self) -> usize {
        self.actions.len()
    }

    fn declared_action_id(&self, index: usize) -> Option<&str> {
        self.actions.get(index).map(|action| action.id.as_str())
    }

    fn declared_action_name(&self, index: usize) -> Option<&str> {
        self.actions.get(index).map(|action| action.name.as_str())
    }
}

pub(crate) fn scan() -> (Vec<App>, Option<SharedString>) {
    let catalog = match rmac_apps::discover() {
        Ok(catalog) => catalog,
        Err(error) => {
            return (
                Vec::new(),
                Some(format!("Could not load applications: {error}").into()),
            );
        }
    };
    let catalog = suppress_replaced_applications(catalog);

    #[cfg(target_os = "macos")]
    let categories = {
        let pairs = catalog
            .iter()
            .map(|application| (application.name.clone(), application.source.clone()))
            .collect::<Vec<_>>();
        parallel_categorize(&pairs)
    };
    #[cfg(not(target_os = "macos"))]
    let categories = catalog
        .iter()
        .map(|application| categorize_desktop(&application.categories))
        .collect::<Vec<_>>();

    let apps = catalog
        .into_iter()
        .zip(categories)
        .map(|(application, category)| App {
            first_party: is_first_party(&application.id),
            search_text: format!(
                "{}\n{}",
                application.searchable_text(),
                category.label().to_lowercase()
            ),
            id: application.id,
            name: application.name.into(),
            generic_name: application.generic_name,
            keywords: application.keywords,
            path: application.source,
            icon: application.icon,
            category,
            source_categories: application.categories,
            mime_types: application.mime_types,
            launch: application.launch,
            actions: application.actions,
        })
        .collect();
    (apps, None)
}

fn suppress_replaced_applications(
    catalog: Vec<rmac_apps::Application>,
) -> Vec<rmac_apps::Application> {
    let replaced_names = catalog
        .iter()
        .filter(|application| is_first_party(application.id.as_str()))
        .map(|application| application.name.trim().to_lowercase())
        .collect::<BTreeSet<_>>();
    catalog
        .into_iter()
        .filter(|application| {
            !is_app_drawer(application.id.as_str())
                && (is_first_party(application.id.as_str())
                    || !replaced_names.contains(&application.name.trim().to_lowercase()))
        })
        .collect()
}

fn is_first_party(desktop_id: &str) -> bool {
    let app_id = desktop_id.strip_suffix(".desktop").unwrap_or(desktop_id);
    rmac_apps::identity::ALL.contains(&app_id)
}

fn is_app_drawer(desktop_id: &str) -> bool {
    desktop_id.strip_suffix(".desktop").unwrap_or(desktop_id) == rmac_apps::identity::APP_DRAWER
}

pub(crate) fn signal_change(sender: &async_channel::Sender<()>) {
    let _ = sender.try_send(());
}

#[cfg(target_os = "macos")]
pub(crate) fn cache_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_owned());
    let directory = PathBuf::from(home).join("Library/Caches/rmac-app-drawer");
    std::fs::create_dir_all(&directory).ok();
    directory
}

#[cfg(target_os = "macos")]
pub(crate) fn hydrate_icons(apps: &mut [App]) {
    let cache = cache_dir();
    for app in apps {
        app.icon = extract_icon(&app.name, &app.path, &cache);
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn extract_icon(name: &str, app: &Path, cache: &Path) -> Option<PathBuf> {
    let safe_name: String = name
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect();
    let output = cache.join(format!("{safe_name}.png"));
    if output.exists() {
        return Some(output);
    }
    let icon = icns_path(app)?;
    let converted = Command::new("sips")
        .args([
            "-s",
            "format",
            "png",
            "-Z",
            "128",
            icon.to_str()?,
            "--out",
            output.to_str()?,
        ])
        .output()
        .ok()
        .is_some_and(|result| result.status.success());
    converted
        .then(|| output.clone())
        .filter(|path| path.exists())
}

#[cfg(target_os = "macos")]
fn icns_path(app: &Path) -> Option<PathBuf> {
    let resources = app.join("Contents/Resources");
    let plist = app.join("Contents/Info.plist");

    if let Ok(output) = Command::new("/usr/libexec/PlistBuddy")
        .args([
            "-c",
            "Print :CFBundleIconFile",
            plist.to_string_lossy().as_ref(),
        ])
        .output()
    {
        let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !name.is_empty() {
            let mut path = resources.join(name);
            if path.extension().is_none() {
                path.set_extension("icns");
            }
            if path.exists() {
                return Some(path);
            }
        }
    }

    std::fs::read_dir(&resources)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("icns"))
        .max_by_key(|path| path.metadata().map(|metadata| metadata.len()).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{signal_change, suppress_replaced_applications};

    fn application(id: &str, name: &str) -> rmac_apps::Application {
        rmac_apps::Application {
            id: id.into(),
            name: name.into(),
            generic_name: None,
            keywords: Vec::new(),
            source: PathBuf::from(format!("/usr/share/applications/{id}")),
            icon: None,
            categories: Vec::new(),
            mime_types: Vec::new(),
            launch: rmac_apps::LaunchSpec::Command {
                program: "/usr/bin/true".into(),
                args: Vec::new(),
                working_dir: None,
                terminal: false,
            },
            actions: Vec::new(),
        }
    }

    #[test]
    fn catalog_change_bursts_coalesce() {
        let (sender, receiver) = async_channel::bounded(1);

        signal_change(&sender);
        signal_change(&sender);
        signal_change(&sender);

        assert_eq!(receiver.len(), 1);
    }

    #[test]
    fn first_party_replacements_hide_only_exact_name_duplicates() {
        let catalog = vec![
            application("org.gnome.Nautilus.desktop", "Files"),
            application("org.rmac.Files.desktop", "Files"),
            application("org.example.FileSearch.desktop", "File Search"),
            application("org.example.Terminal.desktop", "Terminal"),
        ];

        let filtered = suppress_replaced_applications(catalog);
        assert_eq!(
            filtered
                .iter()
                .map(|application| application.id.as_str())
                .collect::<Vec<_>>(),
            [
                "org.rmac.Files.desktop",
                "org.example.FileSearch.desktop",
                "org.example.Terminal.desktop"
            ]
        );
    }

    #[test]
    fn duplicate_names_remain_when_rmac_does_not_replace_them() {
        let catalog = vec![
            application("org.example.EditorOne.desktop", "Code"),
            application("org.example.EditorTwo.desktop", "Code"),
        ];

        assert_eq!(suppress_replaced_applications(catalog).len(), 2);
    }

    #[test]
    fn apps_does_not_list_itself() {
        let catalog = vec![
            application("org.rmac.AppDrawer.desktop", "Apps"),
            application("org.rmac.Files.desktop", "Files"),
        ];

        let filtered = suppress_replaced_applications(catalog);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "org.rmac.Files.desktop");
    }
}
mod category;
