use crate::{Command, Control, DismissReason, Interaction, Outcome, PowerValue, View, FOCUS_ORDER};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Popover {
    output: Option<rmac_compositor::OutputId>,
    focused: Option<Control>,
}

impl Popover {
    pub fn output(&self) -> Option<&rmac_compositor::OutputId> {
        self.output.as_ref()
    }

    pub fn focused(&self) -> Option<Control> {
        self.focused
    }

    pub fn is_open(&self) -> bool {
        self.output.is_some()
    }

    /// Toggle the one allowed popover. Reinvoking it on the owning output
    /// closes it; invoking from another output transfers ownership there.
    pub fn toggle(&mut self, output: rmac_compositor::OutputId, view: &View) -> Outcome {
        if self.output.as_ref() == Some(&output) {
            return self.dismiss(DismissReason::Invoker);
        }
        self.output = Some(output);
        self.focused = first_available(view);
        Outcome::Focused(self.focused)
    }

    /// Reconcile keyboard focus after live capability changes.
    pub fn refresh(&mut self, view: &View) -> Outcome {
        if !self.is_open() {
            return Outcome::Unchanged;
        }
        if self
            .focused
            .is_some_and(|control| is_available(view, control))
        {
            return Outcome::Unchanged;
        }
        self.focused = first_available(view);
        Outcome::Focused(self.focused)
    }

    pub fn outside_press(&mut self) -> Outcome {
        self.dismiss(DismissReason::OutsidePress)
    }

    pub fn owner_lost(&mut self) -> Outcome {
        self.dismiss(DismissReason::OwnerLost)
    }

    pub fn interact(&mut self, interaction: Interaction, view: &View) -> Outcome {
        if !self.is_open() {
            return Outcome::Unchanged;
        }
        match interaction {
            Interaction::Escape => self.dismiss(DismissReason::Escape),
            Interaction::FocusNext => self.move_focus(view, 1),
            Interaction::FocusPrevious => self.move_focus(view, -1),
            Interaction::Activate => self
                .focused
                .and_then(|control| activation(view, control))
                .map(Outcome::Command)
                .unwrap_or(Outcome::Unchanged),
            Interaction::Increment => self
                .focused
                .and_then(|control| adjustment(view, control, true))
                .map(Outcome::Command)
                .unwrap_or(Outcome::Unchanged),
            Interaction::Decrement => self
                .focused
                .and_then(|control| adjustment(view, control, false))
                .map(Outcome::Command)
                .unwrap_or(Outcome::Unchanged),
        }
    }

    fn dismiss(&mut self, reason: DismissReason) -> Outcome {
        let Some(output) = self.output.take() else {
            return Outcome::Unchanged;
        };
        self.focused = None;
        Outcome::Dismissed { output, reason }
    }

    fn move_focus(&mut self, view: &View, direction: isize) -> Outcome {
        let available: Vec<_> = FOCUS_ORDER
            .into_iter()
            .filter(|control| is_available(view, *control))
            .collect();
        if available.is_empty() {
            self.focused = None;
            return Outcome::Focused(None);
        }
        let current = self
            .focused
            .and_then(|focused| available.iter().position(|control| *control == focused));
        let index = match (current, direction.is_positive()) {
            (Some(index), true) => (index + 1) % available.len(),
            (Some(0), false) | (None, false) => available.len() - 1,
            (Some(index), false) => index - 1,
            (None, true) => 0,
        };
        self.focused = Some(available[index]);
        Outcome::Focused(self.focused)
    }
}

fn first_available(view: &View) -> Option<Control> {
    FOCUS_ORDER
        .into_iter()
        .find(|control| is_available(view, *control))
}

fn is_available(view: &View, control: Control) -> bool {
    match control {
        Control::Wifi => view.wifi.available,
        Control::Bluetooth => view.bluetooth.available,
        Control::Sound => view.sound.available,
        Control::Power => view.power.available,
        Control::Focus => view.focus.available,
    }
}

fn is_busy(view: &View, control: Control) -> bool {
    match control {
        Control::Wifi => view.wifi.busy,
        Control::Bluetooth => view.bluetooth.busy,
        Control::Sound => view.sound.busy,
        Control::Power => view.power.busy,
        Control::Focus => view.focus.busy,
    }
}

fn activation(view: &View, control: Control) -> Option<Command> {
    if !is_available(view, control) || is_busy(view, control) {
        return None;
    }
    match control {
        Control::Wifi => Some(Command::SetWifiEnabled(!view.wifi.value)),
        Control::Bluetooth => Some(Command::SetBluetoothPowered(!view.bluetooth.value)),
        Control::Sound => Some(Command::SetOutputMuted(!view.sound.value.muted)),
        Control::Power => adjacent_profile(&view.power.value, true).map(Command::SetPowerProfile),
        Control::Focus => Some(Command::SetFocusEnabled(!view.focus.value.enabled)),
    }
}

fn adjustment(view: &View, control: Control, increment: bool) -> Option<Command> {
    if !is_available(view, control) || is_busy(view, control) {
        return None;
    }
    match control {
        Control::Sound => {
            let volume = if increment {
                view.sound.value.volume.saturating_add(5).min(100)
            } else {
                view.sound.value.volume.saturating_sub(5)
            };
            (volume != view.sound.value.volume).then_some(Command::SetOutputVolume(volume))
        }
        Control::Power => {
            adjacent_profile(&view.power.value, increment).map(Command::SetPowerProfile)
        }
        _ => None,
    }
}

fn adjacent_profile(value: &PowerValue, forward: bool) -> Option<rmac_power::PowerProfile> {
    if value.supported.is_empty() {
        return None;
    }
    let current = value.active.and_then(|active| {
        value
            .supported
            .iter()
            .position(|profile| *profile == active)
    });
    let index = match (current, forward) {
        (Some(index), true) => (index + 1) % value.supported.len(),
        (Some(0), false) | (None, false) => value.supported.len() - 1,
        (Some(index), false) => index - 1,
        (None, true) => 0,
    };
    Some(value.supported[index])
}
