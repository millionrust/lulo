//! Update snapshot collection authority.

use super::*;

#[derive(Default)]
pub struct Collector {
    snapshot: Snapshot,
    backend_error: Option<Error>,
    finished: bool,
}

impl Collector {
    pub fn apply(&mut self, event: Event) -> Result<(), Error> {
        if self.finished {
            return Err(Error::new(
                ErrorKind::Protocol,
                "the update service sent data after finishing",
            ));
        }
        match event {
            Event::Package {
                info,
                package_id,
                summary,
            } => {
                let Some(update) = Update::from_packagekit(info, &package_id, &summary) else {
                    return Err(Error::new(
                        ErrorKind::Protocol,
                        "the update service returned an invalid package identifier",
                    ));
                };
                if !self
                    .snapshot
                    .updates
                    .iter()
                    .any(|existing| existing.package_id == update.package_id)
                {
                    if self.snapshot.updates.len() >= MAX_UPDATES {
                        self.snapshot.truncated = true;
                    } else {
                        self.snapshot.updates.push(update);
                    }
                }
            }
            Event::BackendError { code, detail } => {
                self.backend_error = Some(packagekit_error(code, &detail));
            }
            Event::RestartRequired { .. } => {}
            Event::Finished { exit } => {
                self.finished = true;
                finish_exit(exit, self.backend_error.take(), "update check")?;
            }
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<Snapshot, Error> {
        if !self.finished {
            return Err(Error::new(
                ErrorKind::Protocol,
                "the update service ended without a completion signal",
            ));
        }
        self.snapshot
            .updates
            .sort_by(|left, right| left.package_id.cmp(&right.package_id));
        Ok(self.snapshot)
    }
}
