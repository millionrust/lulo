//! Appearance accessibility snapshot projection.

use std::collections::HashSet;

use super::helpers::*;
use super::model::*;

pub fn project_appearance(
    input: AppearanceInput<'_>,
) -> Result<AppearanceAccessibilitySnapshot, AccessibilityProjectionError> {
    let state = if input.loading {
        PaneState::Loading
    } else if input.theme.is_none() {
        PaneState::Unavailable
    } else if input.busy || input.refreshing {
        PaneState::Busy
    } else {
        PaneState::Ready
    };
    let mut budget = TextBudget::default();
    for fixed in [
        APPEARANCE_TITLE,
        REFRESH_ID,
        REFRESH_LABEL,
        LOADING_LABEL,
        APPLYING_LABEL,
        UNAVAILABLE_LABEL,
        HOST_UNAVAILABLE_LABEL,
    ] {
        budget.add_required(fixed, false)?;
    }
    if let Some(error) = input.error {
        budget.add_required(error, true)?;
    }

    let refresh_enabled = !input.loading && !input.busy && !input.refreshing;
    let refresh_action = AccessibleAction {
        id: REFRESH_ID.into(),
        name: REFRESH_LABEL,
        kind: AppearanceAction::Refresh,
        enabled: refresh_enabled,
        selected: false,
    };
    let mut groups = Vec::new();
    let mut authority = None;
    let mut detail = None;
    if let Some(theme) = input.theme.filter(|_| !input.loading) {
        validate_accent(theme.preferences.accent_color)?;
        let enabled = state == PaneState::Ready;
        groups = vec![
            scheme_group(
                theme.preferences.color_scheme,
                enabled,
                state == PaneState::Busy,
            ),
            accent_group(
                theme.preferences.accent_color,
                enabled,
                state == PaneState::Busy,
            )?,
            contrast_group(
                theme.preferences.contrast,
                enabled,
                state == PaneState::Busy,
            ),
            motion_group(theme.preferences.motion, enabled, state == PaneState::Busy),
        ];
        authority = Some(AccessibleAuthority {
            host_preference: if input.host.capabilities.color_scheme {
                input.host.color_scheme.label()
            } else {
                "Not exposed"
            },
            host_preference_exposed: input.host.capabilities.color_scheme,
            effective_appearance: match theme.effective.color_scheme {
                rmac_appearance::ResolvedColorScheme::Light => "Light",
                rmac_appearance::ResolvedColorScheme::Dark => "Dark",
            },
        });
        if let Some(value) = &theme.detail {
            budget.add_required(value, true)?;
            detail = Some(value.clone());
        }
    }

    validate_groups(&groups)?;
    let mut keyboard_order = Vec::new();
    if refresh_action.enabled {
        keyboard_order.push(refresh_action.id.clone());
    }
    keyboard_order.extend(
        groups
            .iter()
            .flat_map(|group| &group.choices)
            .filter(|choice| choice.enabled)
            .map(|choice| choice.id.clone()),
    );
    let mut action_ids = HashSet::with_capacity(keyboard_order.len());
    if keyboard_order
        .iter()
        .any(|id| !action_ids.insert(id.as_str()))
    {
        return Err(AccessibilityProjectionError::DuplicateAction);
    }

    let mut announcements = Vec::new();
    match state {
        PaneState::Loading => push_announcement(
            &mut announcements,
            "appearance-loading",
            LOADING_LABEL,
            LivePoliteness::Polite,
            &mut budget,
        )?,
        PaneState::Unavailable => push_announcement(
            &mut announcements,
            "appearance-unavailable",
            UNAVAILABLE_LABEL,
            LivePoliteness::Assertive,
            &mut budget,
        )?,
        PaneState::Busy => push_announcement(
            &mut announcements,
            "appearance-busy",
            APPLYING_LABEL,
            LivePoliteness::Polite,
            &mut budget,
        )?,
        PaneState::Ready => {}
    }
    if let Some(error) = input.error {
        push_announcement(
            &mut announcements,
            "appearance-error",
            error,
            LivePoliteness::Assertive,
            &mut budget,
        )?;
    }
    if !input.host.available {
        push_announcement(
            &mut announcements,
            "appearance-host-unavailable",
            HOST_UNAVAILABLE_LABEL,
            LivePoliteness::Polite,
            &mut budget,
        )?;
    }

    Ok(AppearanceAccessibilitySnapshot {
        title: APPEARANCE_TITLE,
        state,
        refresh_action,
        groups,
        authority,
        detail,
        initial_focus: keyboard_order.first().cloned(),
        keyboard_order,
        announcements,
    })
}
