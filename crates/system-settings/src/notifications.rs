#[derive(Clone, Copy)]
pub(super) enum NotificationPolicyChange {
    Enabled(bool),
    Banners(bool),
    Sounds(bool),
    Badges(bool),
    History(bool),
    UrgentThroughFocus(bool),
}

pub(super) fn policy_with(
    mut policy: rmac_notifications_store::AppPolicy,
    change: NotificationPolicyChange,
) -> rmac_notifications_store::AppPolicy {
    match change {
        NotificationPolicyChange::Enabled(value) => policy.enabled = value,
        NotificationPolicyChange::Banners(value) => policy.banners = value,
        NotificationPolicyChange::Sounds(value) => policy.sounds = value,
        NotificationPolicyChange::Badges(value) => policy.badges = value,
        NotificationPolicyChange::History(value) => policy.history = value,
        NotificationPolicyChange::UrgentThroughFocus(value) => {
            policy.urgent_through_focus = value;
        }
    }
    policy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_touch_only_the_selected_field() {
        let original = rmac_notifications_store::AppPolicy::default();
        for (change, expected) in [
            (
                NotificationPolicyChange::Enabled(false),
                rmac_notifications_store::AppPolicy {
                    enabled: false,
                    ..original
                },
            ),
            (
                NotificationPolicyChange::Banners(false),
                rmac_notifications_store::AppPolicy {
                    banners: false,
                    ..original
                },
            ),
            (
                NotificationPolicyChange::Sounds(false),
                rmac_notifications_store::AppPolicy {
                    sounds: false,
                    ..original
                },
            ),
            (
                NotificationPolicyChange::Badges(false),
                rmac_notifications_store::AppPolicy {
                    badges: false,
                    ..original
                },
            ),
            (
                NotificationPolicyChange::History(false),
                rmac_notifications_store::AppPolicy {
                    history: false,
                    ..original
                },
            ),
            (
                NotificationPolicyChange::UrgentThroughFocus(false),
                rmac_notifications_store::AppPolicy {
                    urgent_through_focus: false,
                    ..original
                },
            ),
        ] {
            assert_eq!(policy_with(original, change), expected);
        }
    }
}
