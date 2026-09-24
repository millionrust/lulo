//! Deterministic Dock stack (folder/file, kept left of the Trash) mutation.

use super::*;

pub fn apply_stack_command(
    stacks: &[rmac_shell_settings::DockStackEntry],
    command: &StackCommand,
) -> Result<Vec<rmac_shell_settings::DockStackEntry>, StackError> {
    let target = command.kind();
    let mut result = stacks.to_vec();
    let existing = result.iter().position(|entry| &entry.kind == target);
    match command {
        StackCommand::Add(kind) => {
            if existing.is_some() {
                return Err(StackError::AlreadyKept);
            }
            result.push(rmac_shell_settings::DockStackEntry {
                kind: kind.clone(),
                display_as: rmac_shell_settings::DockStackDisplayAs::default(),
                view_content_as: rmac_shell_settings::DockStackViewContentAs::default(),
                sort_by: rmac_shell_settings::DockStackSortBy::default(),
            });
        }
        StackCommand::Remove(_) => {
            let Some(index) = existing else {
                return Err(StackError::NotKept);
            };
            result.remove(index);
        }
        StackCommand::SetDisplayAs { display_as, .. } => {
            let Some(index) = existing else {
                return Err(StackError::NotKept);
            };
            result[index].display_as = *display_as;
        }
        StackCommand::SetViewContentAs {
            view_content_as, ..
        } => {
            let Some(index) = existing else {
                return Err(StackError::NotKept);
            };
            result[index].view_content_as = *view_content_as;
        }
        StackCommand::SetSortBy { sort_by, .. } => {
            let Some(index) = existing else {
                return Err(StackError::NotKept);
            };
            result[index].sort_by = *sort_by;
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmac_shell_settings::{
        DockStackDisplayAs, DockStackEntry, DockStackKind, DockStackSortBy, DockStackViewContentAs,
    };

    fn downloads() -> DockStackEntry {
        DockStackEntry {
            kind: DockStackKind::Downloads,
            display_as: DockStackDisplayAs::Stack,
            view_content_as: DockStackViewContentAs::Automatic,
            sort_by: DockStackSortBy::DateAdded,
        }
    }

    fn projects() -> DockStackEntry {
        DockStackEntry {
            kind: DockStackKind::Path {
                path: "/home/test/Projects".into(),
            },
            display_as: DockStackDisplayAs::Folder,
            view_content_as: DockStackViewContentAs::Grid,
            sort_by: DockStackSortBy::Name,
        }
    }

    #[test]
    fn add_keeps_a_new_stack_with_default_display() {
        let result =
            apply_stack_command(&[], &StackCommand::Add(DockStackKind::Downloads)).unwrap();
        // `downloads()`'s fields already are the type's `#[default]` variants.
        assert_eq!(result, vec![downloads()]);
    }

    #[test]
    fn add_rejects_a_stack_already_kept() {
        let error =
            apply_stack_command(&[downloads()], &StackCommand::Add(DockStackKind::Downloads))
                .unwrap_err();
        assert_eq!(error, StackError::AlreadyKept);
    }

    #[test]
    fn remove_drops_the_matching_entry_and_leaves_others() {
        let result = apply_stack_command(
            &[downloads(), projects()],
            &StackCommand::Remove(DockStackKind::Downloads),
        )
        .unwrap();
        assert_eq!(result, vec![projects()]);
    }

    #[test]
    fn remove_rejects_a_stack_not_kept() {
        let error = apply_stack_command(
            &[projects()],
            &StackCommand::Remove(DockStackKind::Downloads),
        )
        .unwrap_err();
        assert_eq!(error, StackError::NotKept);
    }

    #[test]
    fn set_display_as_mutates_only_the_matching_entry() {
        let result = apply_stack_command(
            &[downloads(), projects()],
            &StackCommand::SetDisplayAs {
                kind: DockStackKind::Downloads,
                display_as: DockStackDisplayAs::Folder,
            },
        )
        .unwrap();
        assert_eq!(result[0].display_as, DockStackDisplayAs::Folder);
        assert_eq!(result[1], projects());
    }

    #[test]
    fn set_view_content_as_mutates_only_the_matching_entry() {
        let result = apply_stack_command(
            &[downloads()],
            &StackCommand::SetViewContentAs {
                kind: DockStackKind::Downloads,
                view_content_as: DockStackViewContentAs::List,
            },
        )
        .unwrap();
        assert_eq!(result[0].view_content_as, DockStackViewContentAs::List);
    }

    #[test]
    fn set_sort_by_mutates_only_the_matching_entry() {
        let result = apply_stack_command(
            &[projects()],
            &StackCommand::SetSortBy {
                kind: DockStackKind::Path {
                    path: "/home/test/Projects".into(),
                },
                sort_by: DockStackSortBy::Kind,
            },
        )
        .unwrap();
        assert_eq!(result[0].sort_by, DockStackSortBy::Kind);
    }

    #[test]
    fn mutating_a_stack_not_kept_is_rejected() {
        let error = apply_stack_command(
            &[],
            &StackCommand::SetSortBy {
                kind: DockStackKind::Downloads,
                sort_by: DockStackSortBy::Kind,
            },
        )
        .unwrap_err();
        assert_eq!(error, StackError::NotKept);
    }
}
