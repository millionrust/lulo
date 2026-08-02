//! Reviewed install-plan model and collection authority.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Update,
    Install,
    Remove,
    Obsolete,
    Reinstall,
    Downgrade,
}

impl ChangeKind {
    pub fn from_packagekit(info: u32) -> Result<Self, Error> {
        match info {
            11 => Ok(Self::Update),
            12 | 27 => Ok(Self::Install),
            13 | 28 => Ok(Self::Remove),
            15 | 29 => Ok(Self::Obsolete),
            19 => Ok(Self::Reinstall),
            20 | 30 => Ok(Self::Downgrade),
            23 => Err(Error::new(
                ErrorKind::Trust,
                "the update plan contains an untrusted package",
            )),
            _ => Err(Error::new(
                ErrorKind::Protocol,
                "the update simulation returned an unknown package action",
            )),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Update => "Update",
            Self::Install => "Install",
            Self::Remove => "Remove",
            Self::Obsolete => "Replace",
            Self::Reinstall => "Reinstall",
            Self::Downgrade => "Downgrade",
        }
    }

    pub fn is_destructive(self) -> bool {
        matches!(self, Self::Remove | Self::Obsolete | Self::Downgrade)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedChange {
    pub package_id: String,
    pub name: String,
    pub version: String,
    pub summary: String,
    pub kind: ChangeKind,
}

impl PlannedChange {
    fn from_packagekit(info: u32, package_id: &str, summary: &str) -> Result<Self, Error> {
        let kind = ChangeKind::from_packagekit(info)?;
        let (name, version) = package_identity(package_id).ok_or_else(|| {
            Error::new(
                ErrorKind::Protocol,
                "the update simulation returned an invalid package identifier",
            )
        })?;
        Ok(Self {
            package_id: package_id.to_string(),
            name,
            version,
            summary: bounded_text(summary),
            kind,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallPlan {
    pub requested: Vec<Update>,
    pub changes: Vec<PlannedChange>,
    pub restart: RestartRequirement,
}

impl InstallPlan {
    pub fn requested_ids(&self) -> Vec<String> {
        self.requested
            .iter()
            .map(|update| update.package_id.clone())
            .collect()
    }

    pub fn has_destructive_changes(&self) -> bool {
        self.changes
            .iter()
            .any(|change| change.kind.is_destructive())
    }

    pub fn change_count(&self, kind: ChangeKind) -> usize {
        self.changes
            .iter()
            .filter(|change| change.kind == kind)
            .count()
    }
}

pub struct PlanCollector {
    requested: Vec<Update>,
    changes: Vec<PlannedChange>,
    restart: RestartRequirement,
    backend_error: Option<Error>,
    finished: bool,
}

impl PlanCollector {
    pub fn new(snapshot: &Snapshot) -> Result<Self, Error> {
        if !snapshot.install_supported {
            return Err(Error::new(
                ErrorKind::Unavailable,
                snapshot
                    .install_unavailable_reason
                    .as_deref()
                    .unwrap_or("the update backend cannot install updates"),
            ));
        }
        if snapshot.truncated {
            return Err(Error::new(
                ErrorKind::Protocol,
                "the complete update set is too large to confirm safely",
            ));
        }
        let mut requested = snapshot.installable_updates().cloned().collect::<Vec<_>>();
        requested.sort_by(|left, right| left.package_id.cmp(&right.package_id));
        if requested.is_empty() {
            return Err(Error::new(
                ErrorKind::Stale,
                "no installable updates remain",
            ));
        }
        Ok(Self {
            requested,
            changes: Vec::new(),
            restart: RestartRequirement::None,
            backend_error: None,
            finished: false,
        })
    }

    pub fn apply(&mut self, event: Event) -> Result<(), Error> {
        if self.finished {
            return Err(Error::new(
                ErrorKind::Protocol,
                "the update simulation sent data after finishing",
            ));
        }
        match event {
            Event::Package {
                info,
                package_id,
                summary,
            } => {
                let change = PlannedChange::from_packagekit(info, &package_id, &summary)?;
                if !self.changes.iter().any(|existing| {
                    existing.package_id == change.package_id && existing.kind == change.kind
                }) {
                    if self.changes.len() >= MAX_PLAN_CHANGES {
                        return Err(Error::new(
                            ErrorKind::Protocol,
                            "the update simulation is too large to confirm safely",
                        ));
                    }
                    self.changes.push(change);
                }
            }
            Event::BackendError { code, detail } => {
                self.backend_error = Some(packagekit_error(code, &detail));
            }
            Event::RestartRequired { kind, .. } => {
                self.restart = self.restart.max(RestartRequirement::from_packagekit(kind));
            }
            Event::Finished { exit } => {
                self.finished = true;
                finish_exit(exit, self.backend_error.take(), "update simulation")?;
            }
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<InstallPlan, Error> {
        if !self.finished {
            return Err(Error::new(
                ErrorKind::Protocol,
                "the update simulation ended without a completion signal",
            ));
        }
        if self.changes.is_empty() {
            return Err(Error::new(
                ErrorKind::Stale,
                "the update simulation found no package changes",
            ));
        }
        self.changes.sort_by(|left, right| {
            left.package_id
                .cmp(&right.package_id)
                .then_with(|| change_rank(left.kind).cmp(&change_rank(right.kind)))
        });
        Ok(InstallPlan {
            requested: self.requested,
            changes: self.changes,
            restart: self.restart,
        })
    }
}
