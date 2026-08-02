use chrono::{DateTime, FixedOffset};

use crate::labels::{bounded, nonempty};
use crate::model::MAX_WORKSPACE_CHARACTERS;
use crate::{
    active_app_name, clock_label, delay_until_next_clock_update, indicator_labels, Content,
    LocaleHourCycle, Projection, Surface, SystemMark, Update, BAR_HEIGHT,
};

#[derive(Default)]
pub struct State {
    accepted: Option<Projection>,
}

impl State {
    pub fn apply(
        &mut self,
        status: &rmac_shell_status::Snapshot,
        now: DateTime<FixedOffset>,
        locale_hour_cycle: LocaleHourCycle,
    ) -> Update {
        let projection = project(status, now, locale_hour_cycle);
        let redraw = self.accepted.as_ref() != Some(&projection);
        if redraw {
            self.accepted = Some(projection.clone());
        }
        Update {
            projection,
            redraw,
            next_clock_update: delay_until_next_clock_update(
                now.timestamp_millis(),
                status.clock.show_seconds,
            ),
        }
    }
}

pub fn project(
    status: &rmac_shell_status::Snapshot,
    now: DateTime<FixedOffset>,
    locale_hour_cycle: LocaleHourCycle,
) -> Projection {
    let mut surfaces = status
        .outputs
        .iter()
        .filter(|output| {
            output.logical_size.is_valid()
                && output.logical_size.width > 0.0
                && output.logical_size.height > 0.0
                && output.scale.is_finite()
                && output.scale > 0.0
        })
        .map(|output| Surface {
            output: output.id.clone(),
            logical_width: output.logical_size.width,
            logical_height: BAR_HEIGHT,
            scale: output.scale,
            exclusive_zone: BAR_HEIGHT,
            keyboard_interactive: false,
        })
        .collect::<Vec<_>>();
    surfaces.sort_by(|left, right| left.output.cmp(&right.output));
    Projection {
        surfaces,
        content: Content {
            system_mark: SystemMark::default(),
            active_app: active_app_name(status),
            workspace: status
                .clock
                .show_workspace
                .then_some(status.focused.workspace_label.as_deref())
                .flatten()
                .and_then(nonempty)
                .map(|label| bounded(label, MAX_WORKSPACE_CHARACTERS)),
            clock: clock_label(now, &status.clock, locale_hour_cycle),
            indicators: indicator_labels(status),
        },
    }
}
