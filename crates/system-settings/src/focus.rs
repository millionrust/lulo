pub(super) struct FocusLoad {
    pub(super) configuration: rmac_focus::Config,
    pub(super) state: rmac_focus_linux::client::Snapshot,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum FocusCurrentAction {
    None,
    TurnOffManual,
    EditSchedule(rmac_focus::ScheduleId),
}

pub(super) fn current_action(source: Option<&rmac_focus::ActivationSource>) -> FocusCurrentAction {
    match source {
        Some(rmac_focus::ActivationSource::Manual) => FocusCurrentAction::TurnOffManual,
        Some(rmac_focus::ActivationSource::Schedule(schedule_id)) => {
            FocusCurrentAction::EditSchedule(schedule_id.clone())
        }
        None => FocusCurrentAction::None,
    }
}

pub(super) fn load() -> std::result::Result<FocusLoad, rmac_focus_linux::client::Error> {
    let snapshot = rmac_focus_linux::client::settings()?;
    Ok(FocusLoad {
        configuration: snapshot.configuration,
        state: snapshot.state,
    })
}

pub(super) const DAYS: [(rmac_focus::Weekday, &str); 7] = [
    (rmac_focus::Weekday::Monday, "M"),
    (rmac_focus::Weekday::Tuesday, "T"),
    (rmac_focus::Weekday::Wednesday, "W"),
    (rmac_focus::Weekday::Thursday, "T"),
    (rmac_focus::Weekday::Friday, "F"),
    (rmac_focus::Weekday::Saturday, "S"),
    (rmac_focus::Weekday::Sunday, "S"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduled_activation_routes_to_its_editor() {
        assert_eq!(current_action(None), FocusCurrentAction::None);
        assert_eq!(
            current_action(Some(&rmac_focus::ActivationSource::Manual)),
            FocusCurrentAction::TurnOffManual
        );
        let schedule_id = rmac_focus::ScheduleId::parse("weekday").unwrap();
        assert_eq!(
            current_action(Some(&rmac_focus::ActivationSource::Schedule(
                schedule_id.clone()
            ))),
            FocusCurrentAction::EditSchedule(schedule_id)
        );
    }
}
