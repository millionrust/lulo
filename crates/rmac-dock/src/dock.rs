//! Dock item projection and interaction authority.

use super::*;

/// One persisted `DockStackEntry` resolved against the live filesystem. The
/// existence check is the caller's I/O to make (a background executor task
/// in the resident Dock, or the dispatch layer's snapshot builder); `Model`
/// itself never touches the filesystem.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedStack {
    pub entry: rmac_shell_settings::DockStackEntry,
    pub available: bool,
}

impl Model {
    pub fn build(
        pinned: &[rmac_shell_settings::AppId],
        settings: &rmac_shell_settings::DockSettings,
        catalog: &[rmac_apps::Application],
        compositor: &rmac_compositor::Snapshot,
    ) -> Self {
        Self::build_inner(pinned, settings, catalog, compositor, None, &[])
    }

    pub fn build_with_places(
        pinned: &[rmac_shell_settings::AppId],
        settings: &rmac_shell_settings::DockSettings,
        catalog: &[rmac_apps::Application],
        compositor: &rmac_compositor::Snapshot,
        places: &rmac_places::Snapshot,
    ) -> Self {
        Self::build_inner(pinned, settings, catalog, compositor, Some(places), &[])
    }

    /// As `build_with_places`, and also projects folder/file stacks kept
    /// left of the Trash. `stacks` is the persisted configuration resolved
    /// against the filesystem by the caller (see `ResolvedStack`).
    pub fn build_with_stacks(
        pinned: &[rmac_shell_settings::AppId],
        settings: &rmac_shell_settings::DockSettings,
        catalog: &[rmac_apps::Application],
        compositor: &rmac_compositor::Snapshot,
        places: &rmac_places::Snapshot,
        stacks: &[ResolvedStack],
    ) -> Self {
        Self::build_inner(pinned, settings, catalog, compositor, Some(places), stacks)
    }

    fn build_inner(
        pinned: &[rmac_shell_settings::AppId],
        settings: &rmac_shell_settings::DockSettings,
        catalog: &[rmac_apps::Application],
        compositor: &rmac_compositor::Snapshot,
        places: Option<&rmac_places::Snapshot>,
        stacks: &[ResolvedStack],
    ) -> Self {
        let applications = catalog_index(catalog);
        let mut windows = window_groups(compositor);
        let mut items = Vec::new();
        let mut represented = BTreeSet::new();

        for pinned_id in pinned {
            let canonical = canonical_app_id(&pinned_id.0);
            if canonical.is_empty() || !represented.insert(canonical.clone()) {
                continue;
            }
            let application = applications.get(&canonical).copied();
            let grouped = windows.remove(&canonical).unwrap_or_default();
            items.push(build_item(&pinned_id.0, application, true, grouped));
        }

        let mut running: Vec<_> = windows
            .into_iter()
            .filter(|(canonical, grouped)| {
                !canonical.is_empty()
                    && !grouped.is_empty()
                    && represented.insert(canonical.clone())
            })
            .map(|(canonical, grouped)| {
                let application = applications.get(&canonical).copied();
                let source_id = application
                    .map(|application| application.id.clone())
                    .or_else(|| grouped.first().map(|window| window.app_id.clone()))
                    .unwrap_or(canonical);
                build_item(&source_id, application, false, grouped)
            })
            .collect();
        // Focus changes must never move a button out from under the pointer.
        // A renderer/runtime may later retain launch order, but the stateless
        // model uses this deterministic identity order rather than recency.
        running.sort_by_key(|item| (item.name.to_lowercase(), item.id.clone()));
        items.extend(running);

        // Parked windows are minimized-window tiles, never running applications
        // (§2.2, §4.11). "Minimize into application icon" (4.15) is not a
        // setting yet, so the macOS default of showing tiles applies.
        let minimized = minimized_items(compositor, &applications);

        Self {
            items,
            special_items: places.map(project_special_items).unwrap_or_default(),
            stacks: project_stacks(stacks, places),
            minimized,
            repeated_click: settings.repeated_click,
        }
    }

    /// Resolve a minimized-tile click against the newest projection.
    pub fn activate_minimized(&self, window: rmac_compositor::WindowId) -> Activation {
        if self.minimized.iter().any(|item| item.window == window) {
            Activation::RestoreWindow { window }
        } else {
            Activation::NoAction
        }
    }

    pub fn activate_special(&self, kind: SpecialItemKind) -> SpecialActivation {
        self.special_items
            .iter()
            .find(|item| item.kind == kind)
            .map(|item| item.activation.clone())
            .unwrap_or_else(|| SpecialActivation::Unavailable {
                kind,
                detail: "the place is not present in the Dock".into(),
            })
    }

    pub fn activate_stack(&self, kind: &rmac_shell_settings::DockStackKind) -> StackActivation {
        self.stacks
            .iter()
            .find(|stack| &stack.kind == kind)
            .map(|stack| stack.activation.clone())
            .unwrap_or_else(|| StackActivation::Unavailable {
                kind: kind.clone(),
                detail: "the stack is not present in the Dock".into(),
            })
    }

    pub fn stack_context_menu(
        &self,
        kind: &rmac_shell_settings::DockStackKind,
    ) -> Option<StackContextMenu> {
        let stack = self.stacks.iter().find(|stack| &stack.kind == kind)?;
        Some(StackContextMenu {
            kind: stack.kind.clone(),
            name: stack.name.clone(),
            open: stack.activation.clone(),
            display_as: stack.display_as,
            view_content_as: stack.view_content_as,
            sort_by: stack.sort_by,
        })
    }

    pub fn special_context_menu(&self, kind: SpecialItemKind) -> Option<SpecialContextMenu> {
        let item = self.special_items.iter().find(|item| item.kind == kind)?;
        Some(SpecialContextMenu {
            kind,
            open: item.activation.clone(),
            empty_trash: match (kind, item.available, item.item_count) {
                (SpecialItemKind::Trash, true, Some(item_count)) if item_count > 0 => {
                    Some(SpecialContextAction::EmptyTrash {
                        expected_item_count: item_count,
                    })
                }
                _ => None,
            },
        })
    }

    pub fn activate(&self, app_id: &str) -> Activation {
        let canonical = canonical_app_id(app_id);
        let Some(item) = self
            .items
            .iter()
            .find(|item| canonical_app_id(&item.id) == canonical)
        else {
            return Activation::Unavailable {
                app_id: app_id.to_owned(),
                detail: "application is not present in the Dock".into(),
            };
        };
        if item.windows.is_empty() {
            // macOS restores the most recently minimized window when every
            // window of the clicked application is in the Dock.
            if let Some(parked) = self.minimized.iter().rev().find(|parked| {
                parked
                    .app_id
                    .as_deref()
                    .is_some_and(|app_id| canonical_app_id(app_id) == canonical)
            }) {
                return Activation::RestoreWindow {
                    window: parked.window,
                };
            }
            return match &item.launch {
                Some(spec) => Activation::Launch {
                    app_id: item.id.clone(),
                    spec: spec.clone(),
                },
                None => Activation::Unavailable {
                    app_id: item.id.clone(),
                    detail: "application is not installed".into(),
                },
            };
        }

        let Some(index) = item.windows.iter().position(|window| window.focused) else {
            // A background application comes forward with all of its windows.
            // `windows` is most recent first, so focus runs in reverse.
            if item.windows.len() == 1 {
                return Activation::FocusWindow(item.windows[0].id);
            }
            return Activation::FocusApplication {
                app_id: item.id.clone(),
                windows: item.windows.iter().rev().map(|window| window.id).collect(),
            };
        };
        // Clicking the frontmost application does nothing on macOS; the
        // optional rmac behaviours below are opt-in settings.
        match self.repeated_click {
            rmac_shell_settings::RepeatedClickBehavior::CycleWindows if item.windows.len() > 1 => {
                Activation::FocusWindow(item.windows[(index + 1) % item.windows.len()].id)
            }
            rmac_shell_settings::RepeatedClickBehavior::CycleWindows
            | rmac_shell_settings::RepeatedClickBehavior::DoNothing => Activation::NoAction,
            rmac_shell_settings::RepeatedClickBehavior::HideApplication => {
                Activation::Unavailable {
                    app_id: item.id.clone(),
                    detail: "niri does not expose application hiding".into(),
                }
            }
        }
    }

    /// The desktop entry that opens files dropped on `app_id`'s tile. As on
    /// macOS only an application that can open documents takes a drop (and
    /// highlights under it); the entry's own field codes decide how.
    pub fn file_drop_handler(&self, app_id: &str) -> Option<PathBuf> {
        let canonical = canonical_app_id(app_id);
        let item = self
            .items
            .iter()
            .find(|item| canonical_app_id(&item.id) == canonical)?;
        if item.mime_types.is_empty() {
            return None;
        }
        item.source.clone()
    }

    pub fn context_menu(&self, app_id: &str) -> Option<ContextMenu> {
        let canonical = canonical_app_id(app_id);
        let index = self
            .items
            .iter()
            .position(|item| canonical_app_id(&item.id) == canonical)?;
        let item = &self.items[index];
        let windows = item
            .windows
            .iter()
            .map(|window| {
                let title = window
                    .title
                    .clone()
                    .filter(|title| !title.trim().is_empty())
                    .unwrap_or_else(|| item.name.clone());
                WindowMenu {
                    id: window.id,
                    title,
                    focused: window.focused,
                    urgent: window.urgent,
                    focus: ContextAction::FocusWindow {
                        app_id: item.id.clone(),
                        window: window.id,
                    },
                    close: ContextAction::CloseWindow {
                        app_id: item.id.clone(),
                        window: window.id,
                    },
                }
            })
            .collect();
        let process_ids = authoritative_process_ids(item);
        let quit = process_ids
            .as_ref()
            .map(|pids| ContextAction::TerminateApplication {
                app_id: item.id.clone(),
                pids: pids.clone(),
                kind: TerminationKind::Quit,
            });
        let force_quit = process_ids.map(|pids| ContextAction::TerminateApplication {
            app_id: item.id.clone(),
            pids,
            kind: TerminationKind::ForceQuit,
        });
        // Hiding parks windows (ADR 0014), so only visible windows count and
        // a hidden application has nothing left to hide.
        let visible = item
            .windows
            .iter()
            .map(|window| window.id)
            .collect::<Vec<_>>();
        let hide = (!visible.is_empty()).then(|| ContextAction::HideApplication {
            app_id: item.id.clone(),
            windows: visible.clone(),
        });
        let hide_others = (!visible.is_empty()).then(|| ContextAction::HideOthers {
            app_id: item.id.clone(),
            windows: self.other_visible_windows(&canonical),
        });
        let show_all_windows = item
            .windows
            .first()
            .map(|window| ContextAction::ShowAllWindows {
                app_id: item.id.clone(),
                window: window.id,
            });
        Some(ContextMenu {
            app_id: item.id.clone(),
            application_name: item.name.clone(),
            open: (!item.running)
                .then(|| {
                    item.launch.clone().map(|spec| ContextAction::LaunchNew {
                        app_id: item.id.clone(),
                        spec,
                    })
                })
                .flatten(),
            application_commands: item
                .actions
                .iter()
                .map(|command| ApplicationCommand {
                    id: command.id.clone(),
                    name: command.name.clone(),
                    action: ContextAction::LaunchNew {
                        app_id: item.id.clone(),
                        spec: command.launch.clone(),
                    },
                })
                .collect(),
            windows,
            show_in_finder: item
                .source
                .clone()
                .map(|source| ContextAction::RevealApplication {
                    app_id: item.id.clone(),
                    source,
                }),
            pin: if item.pinned {
                PinCommand::Unpin {
                    app_id: item.id.clone(),
                }
            } else {
                PinCommand::Pin {
                    app_id: item.id.clone(),
                }
            },
            quit,
            force_quit,
            show_all_windows,
            hide,
            hide_others,
        })
    }

    /// Every visible window of the other applications in the Dock, for
    /// Hide Others.
    fn other_visible_windows(&self, canonical: &str) -> Vec<rmac_compositor::WindowId> {
        let mut windows = self
            .items
            .iter()
            .filter(|item| canonical_app_id(&item.id) != canonical)
            .flat_map(|item| item.windows.iter().map(|window| window.id))
            .collect::<Vec<_>>();
        windows.sort_unstable();
        windows
    }

    /// Revalidates a menu command against the newest Dock projection before a
    /// renderer crosses into an external application, compositor, or process
    /// authority. A menu left open across process/window churn fails closed.
    pub fn authorizes_context_action(&self, action: &ContextAction) -> bool {
        let app_id = match action {
            ContextAction::LaunchNew { app_id, .. }
            | ContextAction::FocusWindow { app_id, .. }
            | ContextAction::CloseWindow { app_id, .. }
            | ContextAction::RevealApplication { app_id, .. }
            | ContextAction::TerminateApplication { app_id, .. }
            | ContextAction::HideApplication { app_id, .. }
            | ContextAction::HideOthers { app_id, .. }
            | ContextAction::ShowAllWindows { app_id, .. } => app_id,
            ContextAction::UpdatePins(command) => command.app_id(),
        };
        let canonical = canonical_app_id(app_id);
        let Some(item) = self
            .items
            .iter()
            .find(|item| canonical_app_id(&item.id) == canonical)
        else {
            return false;
        };
        match action {
            ContextAction::LaunchNew { spec, .. } => {
                item.launch.as_ref() == Some(spec)
                    || item.actions.iter().any(|action| &action.launch == spec)
            }
            ContextAction::FocusWindow { window, .. }
            | ContextAction::CloseWindow { window, .. } => {
                item.windows.iter().any(|candidate| candidate.id == *window)
            }
            ContextAction::RevealApplication { source, .. } => item.source.as_ref() == Some(source),
            ContextAction::TerminateApplication { pids, .. } => {
                authoritative_process_ids(item).as_ref() == Some(pids)
            }
            ContextAction::UpdatePins(PinCommand::Pin { .. }) => !item.pinned,
            ContextAction::UpdatePins(PinCommand::Unpin { .. }) => item.pinned,
            ContextAction::UpdatePins(PinCommand::Move { .. } | PinCommand::MoveTo { .. }) => {
                item.pinned
            }
            ContextAction::HideApplication { windows, .. } => {
                !windows.is_empty()
                    && windows.len() == item.windows.len()
                    && item
                        .windows
                        .iter()
                        .all(|window| windows.contains(&window.id))
            }
            ContextAction::HideOthers { windows, .. } => {
                !item.windows.is_empty() && *windows == self.other_visible_windows(&canonical)
            }
            ContextAction::ShowAllWindows { window, .. } => {
                item.windows.iter().any(|candidate| candidate.id == *window)
            }
        }
    }
}

fn authoritative_process_ids(item: &Item) -> Option<Vec<u32>> {
    if item.windows.is_empty() || item.windows.iter().any(|window| window.pid.is_none()) {
        return None;
    }
    let mut pids = item
        .windows
        .iter()
        .filter_map(|window| window.pid)
        .collect::<Vec<_>>();
    pids.sort_unstable();
    pids.dedup();
    (!pids.is_empty()).then_some(pids)
}

pub(super) fn project_special_items(places: &rmac_places::Snapshot) -> Vec<SpecialItem> {
    let trash = SpecialItem {
        kind: SpecialItemKind::Trash,
        name: "Trash",
        available: places.trash.available,
        item_count: places.trash.available.then_some(places.trash.item_count),
        activation: if places.trash.available {
            SpecialActivation::OpenTrash
        } else {
            SpecialActivation::Unavailable {
                kind: SpecialItemKind::Trash,
                detail: "the desktop Trash authority is unavailable".into(),
            }
        },
    };
    // Files is an ordinary configured application identity and Downloads is an
    // optional folder stack. Neither belongs in an unconditional special-item
    // tail. Until folder stacks have a persisted configuration authority, the
    // only permanent Dock endpoint is the authoritative desktop Trash.
    vec![trash]
}

/// Project persisted, filesystem-resolved stack configuration into the
/// shelf's stack group, kept between the application group and the special
/// items so Trash stays rightmost (§ folder/file stacks).
pub(super) fn project_stacks(
    stacks: &[ResolvedStack],
    places: Option<&rmac_places::Snapshot>,
) -> Vec<StackPlace> {
    stacks
        .iter()
        .map(|resolved| {
            let (name, path) = match &resolved.entry.kind {
                rmac_shell_settings::DockStackKind::Downloads => (
                    "Downloads".to_owned(),
                    places.map(|places| places.downloads.path.clone()),
                ),
                rmac_shell_settings::DockStackKind::Path { path } => {
                    let path_buf = PathBuf::from(path);
                    let name = path_buf
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(str::to_owned)
                        .unwrap_or_else(|| path.clone());
                    (name, Some(path_buf))
                }
            };
            let activation = match (resolved.available, path) {
                (true, Some(path)) => StackActivation::OpenDirectory {
                    kind: resolved.entry.kind.clone(),
                    path,
                },
                _ => StackActivation::Unavailable {
                    kind: resolved.entry.kind.clone(),
                    detail: "the stack's folder is unavailable".into(),
                },
            };
            StackPlace {
                kind: resolved.entry.kind.clone(),
                name,
                available: matches!(activation, StackActivation::OpenDirectory { .. }),
                display_as: resolved.entry.display_as,
                view_content_as: resolved.entry.view_content_as,
                sort_by: resolved.entry.sort_by,
                activation,
            }
        })
        .collect()
}

#[derive(Clone, Debug)]
pub(super) struct GroupedWindow {
    app_id: String,
    window: WindowItem,
}

pub(super) fn catalog_index(
    catalog: &[rmac_apps::Application],
) -> BTreeMap<String, &rmac_apps::Application> {
    let mut index = BTreeMap::new();
    for application in catalog {
        index
            .entry(canonical_app_id(&application.id))
            .or_insert(application);
    }
    index
}

/// Workspaces the shell parks hidden windows on. They are not part of the
/// running-application projection.
pub(super) fn parking_workspaces(
    compositor: &rmac_compositor::Snapshot,
) -> BTreeSet<rmac_compositor::WorkspaceId> {
    compositor
        .workspaces
        .iter()
        .filter(|workspace| {
            workspace.name.as_deref() == Some(rmac_compositor::PARKING_WORKSPACE)
                && !workspace.active
                && !workspace.focused
        })
        .map(|workspace| workspace.id)
        .collect()
}

/// Parked windows as minimized tiles, newest app identity first by window id.
pub(super) fn minimized_items(
    compositor: &rmac_compositor::Snapshot,
    applications: &BTreeMap<String, &rmac_apps::Application>,
) -> Vec<MinimizedItem> {
    let parking = parking_workspaces(compositor);
    let mut minimized: Vec<MinimizedItem> = compositor
        .windows
        .iter()
        .filter(|window| window.workspace.is_some_and(|id| parking.contains(&id)))
        .map(|window| {
            let application = window
                .app_id
                .as_deref()
                .map(canonical_app_id)
                .and_then(|canonical| applications.get(&canonical).copied());
            MinimizedItem {
                window: window.id,
                app_id: window.app_id.clone(),
                title: window.title.clone(),
                icon: application.and_then(|application| application.icon.clone()),
                thumbnail: rmac_compositor::ParkingStore::default_thumbnail_path(window.id),
            }
        })
        .collect();
    minimized.sort_by_key(|item| item.window);
    minimized
}

/// Open/Save panels belong to the app that asked for them, as on macOS, so
/// the separate panel process never appears as an application of its own.
/// Force Quit Applications is a system dialogue with no Dock icon either.
fn is_system_dialog(canonical_app_id: &str) -> bool {
    canonical_app_id == "org.rmac.filechooser"
        || canonical_app_id.starts_with("org.rmac.filechooser.")
        || canonical_app_id == "org.rmac.forcequit"
}

pub(super) fn window_groups(
    compositor: &rmac_compositor::Snapshot,
) -> BTreeMap<String, Vec<GroupedWindow>> {
    let focused_id = compositor.focus.window;
    let parking = parking_workspaces(compositor);
    let mut groups: BTreeMap<String, Vec<GroupedWindow>> = BTreeMap::new();
    for window in compositor
        .windows
        .iter()
        .filter(|window| !window.workspace.is_some_and(|id| parking.contains(&id)))
    {
        let Some(app_id) = window
            .app_id
            .as_ref()
            .filter(|app_id| !app_id.trim().is_empty())
        else {
            continue;
        };
        let canonical = canonical_app_id(app_id);
        if canonical.is_empty() || is_system_dialog(&canonical) {
            continue;
        }
        groups.entry(canonical).or_default().push(GroupedWindow {
            app_id: app_id.clone(),
            window: WindowItem {
                id: window.id,
                title: window.title.clone(),
                pid: window
                    .pid
                    .and_then(|pid| u32::try_from(pid).ok())
                    .filter(|pid| *pid > 0),
                focused: window.focused || focused_id == Some(window.id),
                urgent: window.urgent,
                focus_timestamp: window.focus_timestamp,
            },
        });
    }
    for grouped in groups.values_mut() {
        grouped.sort_by_key(|entry| {
            (
                Reverse(entry.window.focused),
                Reverse(timestamp_key(entry.window.focus_timestamp)),
                entry.window.id,
            )
        });
    }
    groups
}

pub(super) fn build_item(
    source_id: &str,
    application: Option<&rmac_apps::Application>,
    pinned: bool,
    grouped: Vec<GroupedWindow>,
) -> Item {
    let windows: Vec<_> = grouped.into_iter().map(|entry| entry.window).collect();
    let running = !windows.is_empty();
    Item {
        id: application
            .map(|application| application.id.clone())
            .unwrap_or_else(|| source_id.to_owned()),
        name: application
            .map(|application| application.name.clone())
            .unwrap_or_else(|| fallback_name(source_id)),
        icon: application.and_then(|application| application.icon.clone()),
        pinned,
        running,
        active: windows.iter().any(|window| window.focused),
        urgent: windows.iter().any(|window| window.urgent),
        launchable: application.is_some(),
        windows,
        launch: application.map(|application| application.launch.clone()),
        source: application.map(|application| application.source.clone()),
        actions: application
            .map(|application| application.actions.clone())
            .unwrap_or_default(),
        mime_types: application
            .map(|application| application.mime_types.clone())
            .unwrap_or_default(),
    }
}

pub(super) fn canonical_app_id(app_id: &str) -> String {
    app_id
        .trim()
        .strip_suffix(".desktop")
        .unwrap_or(app_id.trim())
        .to_lowercase()
}

pub(super) fn fallback_name(app_id: &str) -> String {
    let canonical = app_id
        .trim()
        .strip_suffix(".desktop")
        .unwrap_or(app_id.trim());
    let name = canonical
        .rsplit(['.', '/', '-'])
        .find(|component| !component.is_empty())
        .unwrap_or("Application");
    let mut characters = name.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => "Application".into(),
    }
}

pub(super) fn timestamp_key(timestamp: Option<rmac_compositor::Timestamp>) -> (u64, u32) {
    timestamp
        .map(|timestamp| (timestamp.seconds, timestamp.nanoseconds))
        .unwrap_or_default()
}
