//! Launcher lifecycle coordinator and keyboard interaction authority.

use super::*;

impl Coordinator {
    pub fn new(
        descriptors: Vec<ProviderDescriptor>,
        policies: BTreeMap<rmac_shell_settings::ProviderId, rmac_shell_settings::ProviderPolicy>,
    ) -> Self {
        Self {
            launcher: rmac_launcher::Launcher::default(),
            descriptors,
            policies,
            next_activation: 0,
            activation: None,
            activation_error: None,
            last_shortcut_timestamp_ms: None,
            application_catalog: CatalogHealth::Unmanaged,
            application_catalog_revision: 0,
        }
    }

    /// Open synchronously. The caller must apply `focus` before it schedules
    /// provider work, so an empty-query filesystem search cannot delay typing.
    pub fn open(&mut self) -> OpenEffect {
        self.activation = None;
        self.activation_error = None;
        OpenEffect {
            request: self.launcher.open(&self.descriptors, &self.policies),
            focus: FocusTarget::Query,
        }
    }

    pub fn set_query(&mut self, query: impl Into<String>) -> Option<Request> {
        if self.activation.is_some() {
            return None;
        }
        self.activation_error = None;
        self.launcher
            .set_query(query, &self.descriptors, &self.policies)
    }

    /// Reapply provider privacy policy immediately. If the overlay is open,
    /// this cancels the old generation before newly admitted providers run.
    pub fn set_policies(
        &mut self,
        policies: BTreeMap<rmac_shell_settings::ProviderId, rmac_shell_settings::ProviderPolicy>,
    ) -> Option<Request> {
        if self.policies == policies {
            return None;
        }
        self.policies = policies;
        if self.launcher.is_open() {
            let query = self.launcher.session().query().to_owned();
            self.launcher
                .set_query(query, &self.descriptors, &self.policies)
        } else {
            None
        }
    }

    /// Replace the complete provider environment after an authoritative
    /// settings/scope reload. An open query is cancelled and reissued once.
    pub fn set_environment(
        &mut self,
        descriptors: Vec<ProviderDescriptor>,
        policies: BTreeMap<rmac_shell_settings::ProviderId, rmac_shell_settings::ProviderPolicy>,
    ) -> Option<Request> {
        if self.descriptors == descriptors && self.policies == policies {
            return None;
        }
        self.descriptors = descriptors;
        self.policies = policies;
        if self.launcher.is_open() {
            let query = self.launcher.session().query().to_owned();
            self.launcher
                .set_query(query, &self.descriptors, &self.policies)
        } else {
            None
        }
    }

    pub fn select(&mut self, id: &ResultId) -> bool {
        self.launcher.is_open() && self.launcher.session_mut().select(id)
    }

    pub fn activate_selected(&mut self, mode: ActivationMode) -> KeyEffect {
        if !self.launcher.is_open() {
            KeyEffect::None
        } else {
            self.activation(mode)
        }
    }

    pub fn apply(&mut self, batch: Batch) -> bool {
        self.launcher.is_open()
            && self
                .launcher
                .session_mut()
                .apply(batch.generation, batch.provider, batch.results)
    }

    pub fn apply_catalog(&mut self, update: CatalogUpdate) -> CatalogEffect {
        if update.revision < self.application_catalog_revision {
            return CatalogEffect {
                visible: false,
                request: None,
            };
        }
        let visible = self.application_catalog != update.health;
        let revision_changed =
            update.changed && update.revision > self.application_catalog_revision;
        self.application_catalog = update.health;
        self.application_catalog_revision = self.application_catalog_revision.max(update.revision);
        let request = if revision_changed && self.launcher.is_open() && self.activation.is_none() {
            let query = self.launcher.session().query().to_owned();
            self.launcher
                .set_query(query, &self.descriptors, &self.policies)
        } else {
            None
        };
        CatalogEffect {
            visible: visible || request.is_some(),
            request,
        }
    }

    /// Handle only the stable launcher shortcut. Duplicate or older portal
    /// timestamps are ignored, and a fresh activation toggles the overlay.
    pub fn handle_shortcut(&mut self, event: &rmac_shortcuts::Event) -> ShortcutEffect {
        let rmac_shortcuts::Event::Activated { id, timestamp_ms } = event else {
            return ShortcutEffect::None;
        };
        if id.0 != "launcher"
            || self
                .last_shortcut_timestamp_ms
                .is_some_and(|last| *timestamp_ms <= last)
        {
            return ShortcutEffect::None;
        }
        self.last_shortcut_timestamp_ms = Some(*timestamp_ms);
        if self.launcher.is_open() {
            self.activation = None;
            self.activation_error = None;
            self.launcher.escape();
            ShortcutEffect::Dismissed
        } else {
            ShortcutEffect::Open(self.open())
        }
    }

    pub fn handle_key(&mut self, command: KeyCommand) -> KeyEffect {
        if !self.launcher.is_open() {
            return KeyEffect::None;
        }
        match command {
            KeyCommand::ArrowDown => {
                let before = self.launcher.session().selected().cloned();
                self.launcher
                    .session_mut()
                    .move_selection(MoveSelection::Next);
                if before != self.launcher.session().selected().cloned() {
                    KeyEffect::SelectionChanged
                } else {
                    KeyEffect::None
                }
            }
            KeyCommand::ArrowUp => {
                let before = self.launcher.session().selected().cloned();
                self.launcher
                    .session_mut()
                    .move_selection(MoveSelection::Previous);
                if before != self.launcher.session().selected().cloned() {
                    KeyEffect::SelectionChanged
                } else {
                    KeyEffect::None
                }
            }
            KeyCommand::Return => self.activation(ActivationMode::Primary),
            KeyCommand::AlternateReturn => self.activation(ActivationMode::Alternate),
            KeyCommand::Escape => {
                self.activation = None;
                self.activation_error = None;
                self.launcher.escape();
                KeyEffect::Dismissed
            }
        }
    }

    pub fn finish_activation(
        &mut self,
        result: Result<rmac_launcher_system::Receipt, rmac_launcher_system::Error>,
    ) -> bool {
        let id = match &result {
            Ok(receipt) => receipt.activation,
            Err(error) => error.activation,
        };
        if self.activation != Some(id) || !self.launcher.is_open() {
            return false;
        }
        self.activation = None;
        match result {
            Ok(_) => {
                self.activation_error = None;
                self.launcher.escape();
            }
            Err(error) => self.activation_error = Some(error.to_string()),
        }
        true
    }

    pub fn snapshot(&self) -> Snapshot {
        if !self.launcher.is_open() {
            return Snapshot {
                open: false,
                query: String::new(),
                phase: Phase::Closed,
                rows: Vec::new(),
                pending_providers: 0,
                failed_providers: 0,
                application_catalog: self.application_catalog.clone(),
                announcement: None,
            };
        }
        let session = self.launcher.session();
        let pending = session.pending().len();
        let failed = session.errors().len();
        let selected = session.selected();
        let rows: Vec<_> = session
            .results()
            .iter()
            .map(|ranked| Row {
                id: ranked.result.id.clone(),
                category: ranked.result.category,
                category_label: ranked.result.category.label(),
                title: ranked.result.title.clone(),
                subtitle: ranked.result.subtitle.clone(),
                icon: ranked.result.icon.clone(),
                selected: selected == Some(&ranked.result.id),
                has_alternate: ranked.result.alternate.is_some(),
            })
            .collect();
        let catalog_starting = self.application_catalog == CatalogHealth::Starting;
        let catalog_unavailable = self.application_catalog == CatalogHealth::Unavailable;
        let phase = if self.activation.is_some() {
            Phase::Activating
        } else if self.activation_error.is_some() {
            Phase::ActivationFailed
        } else if rows.is_empty() && (pending > 0 || catalog_starting) {
            Phase::Loading
        } else if !rows.is_empty() {
            Phase::Results {
                still_searching: pending > 0,
                degraded: failed > 0 || catalog_unavailable,
            }
        } else if failed > 0 || catalog_unavailable {
            Phase::Unavailable
        } else {
            Phase::Empty
        };
        let announcement = announcement(&phase, &rows, self.activation_error.as_deref());
        Snapshot {
            open: true,
            query: session.query().to_owned(),
            phase,
            rows,
            pending_providers: pending,
            failed_providers: failed,
            application_catalog: self.application_catalog.clone(),
            announcement,
        }
    }

    fn activation(&mut self, mode: ActivationMode) -> KeyEffect {
        if self.activation.is_some() {
            return KeyEffect::None;
        }
        let Some(action) = self.launcher.session().activation(mode) else {
            return KeyEffect::None;
        };
        self.next_activation = self.next_activation.wrapping_add(1).max(1);
        let id = rmac_launcher_system::ActivationId(self.next_activation);
        self.activation = Some(id);
        self.activation_error = None;
        KeyEffect::Activate(Activation { id, action })
    }
}

pub(super) fn announcement(
    phase: &Phase,
    rows: &[Row],
    activation_error: Option<&str>,
) -> Option<String> {
    match phase {
        Phase::Closed => None,
        Phase::Loading => Some("Searching".into()),
        Phase::Results { .. } => {
            let selected = rows.iter().position(|row| row.selected)?;
            let row = &rows[selected];
            Some(format!(
                "{}, {}, {} of {} results",
                row.title,
                row.category_label,
                selected + 1,
                rows.len()
            ))
        }
        Phase::Empty => Some("No results".into()),
        Phase::Unavailable => Some("Search providers are unavailable".into()),
        Phase::Activating => Some("Opening selection".into()),
        Phase::ActivationFailed => activation_error.map(str::to_owned),
    }
}
