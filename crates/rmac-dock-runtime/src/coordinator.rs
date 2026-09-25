//! Coherent Dock snapshot projection and source-state reduction.

use super::*;

impl Coordinator {
    pub fn snapshot(&self) -> Snapshot {
        let compositor = self.compositor.snapshot();
        let model = self.places.as_ref().map_or_else(
            || {
                rmac_dock::Model::build(
                    &self.settings.pinned_apps,
                    &self.settings.dock,
                    &self.catalog,
                    &compositor,
                )
            },
            |places| {
                let stacks = resolve_stacks(&self.settings.dock_stacks, places);
                rmac_dock::Model::build_with_stacks(
                    &self.settings.pinned_apps,
                    &self.settings.dock,
                    &self.catalog,
                    &compositor,
                    places,
                    &stacks,
                )
            },
        );
        let model = model.with_recent_applications(&self.recents, &self.catalog, &self.superseded);
        let outputs = rmac_dock::surface_outputs(
            &compositor,
            &self.settings.dock.outputs,
            self.primary_output.as_ref(),
        );
        let surface_plan = rmac_dock::surface_descriptions(
            &compositor,
            &self.settings.dock,
            self.primary_output.as_ref(),
            self.reduced_motion,
        );
        let content = rmac_dock::presentation::ShelfContent::project(&model);
        let content = if self.settings.dock.show_running_indicators {
            content
        } else {
            content.without_activity_indicators()
        };
        let overview_visible = compositor.overview_visible;
        Snapshot {
            compositor,
            settings: self.settings.dock.clone(),
            model,
            content,
            outputs,
            surface_plan,
            overview_visible,
            reduced_motion: self.reduced_motion,
            health: self.health.clone(),
        }
    }

    /// Record running apps that are not kept in the Dock in the recent-apps
    /// section. Only installed apps are recorded, since only they can be
    /// shown again after they quit.
    fn advance_recents(&mut self) {
        if !self.settings.dock.show_recent_apps {
            if !self.recents.is_empty() {
                self.recents.clear();
                self.recents_dirty = true;
            }
            return;
        }
        let model = rmac_dock::Model::build(
            &self.settings.pinned_apps,
            &self.settings.dock,
            &self.catalog,
            &self.compositor.snapshot(),
        );
        let running: Vec<String> = model
            .items
            .iter()
            .filter(|item| !item.pinned && item.running && item.launchable)
            .map(|item| item.id.clone())
            .collect();
        let pinned: Vec<String> = self
            .settings
            .pinned_apps
            .iter()
            .map(|app| app.0.clone())
            .collect();
        let next = rmac_dock::recents::advance(&self.recents, &pinned, &running);
        if next != self.recents {
            self.recents = next;
            self.recents_dirty = true;
        }
    }

    /// Load the persisted section before the first snapshot.
    pub fn restore_recents(&mut self, recents: Vec<String>) {
        self.recents = recents;
    }

    /// The section to persist, once per change.
    pub fn take_dirty_recents(&mut self) -> Option<Vec<String>> {
        std::mem::take(&mut self.recents_dirty).then(|| self.recents.clone())
    }

    pub fn ready(&self) -> bool {
        !matches!(self.health.compositor, SourceHealth::Starting)
            && !matches!(self.health.settings, SourceHealth::Starting)
            && !matches!(self.health.catalog, SourceHealth::Starting)
            && !matches!(self.health.places, SourceHealth::Starting)
            && !matches!(self.health.appearance, SourceHealth::Starting)
            && (!matches!(
                self.settings.dock.outputs,
                rmac_shell_settings::OutputScope::Primary
            ) || !matches!(self.health.displays, SourceHealth::Starting))
    }

    pub fn apply_compositor(&mut self, event: rmac_compositor::Event) -> bool {
        let before = self.snapshot();
        match &event {
            rmac_compositor::Event::ConnectionChanged { state } => {
                self.health.compositor = match state {
                    rmac_compositor::ConnectionState::Connected => SourceHealth::Healthy,
                    rmac_compositor::ConnectionState::Connecting => SourceHealth::Starting,
                    rmac_compositor::ConnectionState::Disconnected => SourceHealth::Unavailable {
                        detail: "niri compositor events are disconnected".into(),
                    },
                    rmac_compositor::ConnectionState::Reconnecting => SourceHealth::Unavailable {
                        detail: "niri compositor events are reconnecting".into(),
                    },
                };
            }
            _ => self.health.compositor = SourceHealth::Healthy,
        }
        self.compositor.apply(event);
        if self.ready() {
            self.advance_recents();
        }
        before != self.snapshot()
    }

    pub fn apply_settings(
        &mut self,
        result: Result<rmac_shell_settings::ShellSettings, String>,
    ) -> bool {
        let before = self.snapshot();
        match result {
            Ok(settings) => {
                self.settings = settings;
                self.health.settings = SourceHealth::Healthy;
                if self.ready() {
                    self.advance_recents();
                }
            }
            Err(detail) => self.health.settings = SourceHealth::Unavailable { detail },
        }
        before != self.snapshot()
    }

    pub fn apply_catalog(&mut self, result: Result<Vec<rmac_apps::Application>, String>) -> bool {
        let before = self.snapshot();
        match result {
            Ok(catalog) => {
                self.catalog = catalog;
                // Refreshed alongside the catalog rather than per snapshot: it
                // is packaging state that only changes on an rmac-apps
                // upgrade, not on every render.
                self.superseded = rmac_apps::superseded_desktop_ids();
                self.health.catalog = SourceHealth::Healthy;
            }
            Err(detail) => self.health.catalog = SourceHealth::Unavailable { detail },
        }
        before != self.snapshot()
    }

    pub fn apply_places(&mut self, result: Result<rmac_places_system::Report, String>) -> bool {
        let before = self.snapshot();
        match result {
            Ok(report) => {
                self.places = Some(report.snapshot);
                self.health.places = if report.warnings.is_empty() {
                    SourceHealth::Healthy
                } else {
                    SourceHealth::Unavailable {
                        detail: "one or more user-place authorities are unavailable".into(),
                    }
                };
            }
            Err(detail) => self.health.places = SourceHealth::Unavailable { detail },
        }
        before != self.snapshot()
    }

    pub fn apply_appearance(&mut self, result: Result<bool, String>) -> bool {
        let before = self.snapshot();
        match result {
            Ok(reduced_motion) => {
                self.reduced_motion = reduced_motion;
                self.health.appearance = SourceHealth::Healthy;
            }
            Err(detail) => self.health.appearance = SourceHealth::Unavailable { detail },
        }
        before != self.snapshot()
    }

    pub fn set_primary_output(&mut self, output: Option<rmac_compositor::OutputId>) -> bool {
        let before = self.snapshot();
        self.primary_output = output;
        before != self.snapshot()
    }

    pub fn apply_primary_output(
        &mut self,
        result: Result<Option<rmac_compositor::OutputId>, String>,
    ) -> bool {
        let before = self.snapshot();
        match result {
            Ok(output) => {
                self.primary_output = output;
                self.health.displays = SourceHealth::Healthy;
            }
            Err(detail) => self.health.displays = SourceHealth::Unavailable { detail },
        }
        before != self.snapshot()
    }
}

/// Resolve persisted `DockStackEntry` configuration against the live
/// filesystem (existence only, no directory listing). A folder/file
/// stack's path comes from the resolved Downloads place for the special
/// case, or straight from the persisted path otherwise.
fn resolve_stacks(
    stacks: &[rmac_shell_settings::DockStackEntry],
    places: &rmac_places::Snapshot,
) -> Vec<rmac_dock::ResolvedStack> {
    stacks
        .iter()
        .map(|entry| {
            let path = match &entry.kind {
                rmac_shell_settings::DockStackKind::Downloads => places.downloads.path.clone(),
                rmac_shell_settings::DockStackKind::Path { path } => std::path::PathBuf::from(path),
            };
            rmac_dock::ResolvedStack {
                entry: entry.clone(),
                available: path.is_dir(),
            }
        })
        .collect()
}
