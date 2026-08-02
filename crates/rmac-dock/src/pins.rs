//! Deterministic pinned-application mutation.

use super::*;

pub fn apply_pin_command(
    pinned: &[rmac_shell_settings::AppId],
    command: &PinCommand,
) -> Result<Vec<rmac_shell_settings::AppId>, PinError> {
    let canonical = canonical_app_id(command.app_id());
    if canonical.is_empty() {
        return Err(PinError::InvalidIdentity);
    }
    let mut result = pinned.to_vec();
    let existing = result
        .iter()
        .position(|app_id| canonical_app_id(&app_id.0) == canonical);
    match command {
        PinCommand::Pin { app_id } => {
            if existing.is_none() {
                result.push(rmac_shell_settings::AppId(app_id.clone()));
            }
        }
        PinCommand::Unpin { app_id } => {
            let Some(index) = existing else {
                return Err(PinError::NotPinned {
                    app_id: app_id.clone(),
                });
            };
            result.remove(index);
        }
        PinCommand::Move { app_id, direction } => {
            let Some(index) = existing else {
                return Err(PinError::NotPinned {
                    app_id: app_id.clone(),
                });
            };
            let destination = match direction {
                MoveDirection::Left => index.saturating_sub(1),
                MoveDirection::Right => (index + 1).min(result.len() - 1),
            };
            if destination != index {
                result.swap(index, destination);
            }
        }
        PinCommand::MoveTo {
            app_id,
            index: destination,
        } => {
            let Some(index) = existing else {
                return Err(PinError::NotPinned {
                    app_id: app_id.clone(),
                });
            };
            let destination = (*destination).min(result.len() - 1);
            if destination != index {
                let moved = result.remove(index);
                result.insert(destination, moved);
            }
        }
    }
    Ok(result)
}
