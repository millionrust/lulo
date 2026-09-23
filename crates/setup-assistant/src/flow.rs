//! The Setup Assistant page flow: which pages exist, how Continue, Back,
//! Set Up Later and Skip Setup move between them, and when the flow is over.
//! Pure, so every transition is unit-tested.

/// One page, in the order the flow shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Step {
    Welcome,
    LanguageRegion,
    Keyboard,
    WiFi,
    Account,
    Appearance,
    Tips,
    Privacy,
    Done,
}

impl Step {
    pub const ALL: [Self; 9] = [
        Self::Welcome,
        Self::LanguageRegion,
        Self::Keyboard,
        Self::WiFi,
        Self::Account,
        Self::Appearance,
        Self::Tips,
        Self::Privacy,
        Self::Done,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Welcome => "Welcome",
            Self::LanguageRegion => "Language & Region",
            Self::Keyboard => "Keyboard",
            Self::WiFi => "Choose a Wi-Fi Network",
            Self::Account => "Your Account",
            Self::Appearance => "Choose Your Look",
            Self::Tips => "Find Your Way Around",
            Self::Privacy => "Your Privacy",
            Self::Done => "You’re All Set",
        }
    }

    /// Pages that change a setting offer "Set Up Later", which moves on
    /// without applying anything.
    pub fn can_set_up_later(self) -> bool {
        matches!(
            self,
            Self::LanguageRegion | Self::Keyboard | Self::WiFi | Self::Account | Self::Appearance
        )
    }
}

/// What the computer offers when the assistant starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Availability {
    /// A Wi-Fi adapter exists and no Wi-Fi network is connected yet.
    pub wifi_needed: bool,
    /// AccountsService answers, so the name and picture can be changed.
    pub accounts: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Continue (or Get Started): the page's changes, if any, were applied.
    Continue,
    Back,
    /// Set Up Later on the current page.
    SetUpLater,
    /// Skip Setup on the Welcome page.
    SkipSetup,
    /// The window was closed.
    Close,
}

/// How the flow ended. Either way the first-login marker is written, so the
/// assistant runs once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completion {
    Finished,
    Skipped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The event does not apply here.
    Ignored,
    Moved(Step),
    Ended(Completion),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flow {
    steps: Vec<Step>,
    index: usize,
    postponed: Vec<Step>,
    ended: Option<Completion>,
}

impl Flow {
    pub fn new(availability: Availability) -> Self {
        let steps = Step::ALL
            .into_iter()
            .filter(|step| match step {
                Step::WiFi => availability.wifi_needed,
                Step::Account => availability.accounts,
                _ => true,
            })
            .collect();
        Self {
            steps,
            index: 0,
            postponed: Vec::new(),
            ended: None,
        }
    }

    pub fn step(&self) -> Step {
        self.steps[self.index]
    }

    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// Pages the user chose to set up later.
    pub fn postponed(&self) -> &[Step] {
        &self.postponed
    }

    pub fn ended(&self) -> Option<Completion> {
        self.ended
    }

    pub fn can_go_back(&self) -> bool {
        self.ended.is_none() && self.index > 0
    }

    pub fn handle(&mut self, event: Event) -> Outcome {
        if self.ended.is_some() {
            return Outcome::Ignored;
        }
        match event {
            Event::Continue => {
                if self.step() == Step::Done {
                    self.end(Completion::Finished)
                } else {
                    self.postponed
                        .retain(|step| *step != self.steps[self.index]);
                    self.advance()
                }
            }
            Event::Back => {
                if self.index == 0 {
                    Outcome::Ignored
                } else {
                    self.index -= 1;
                    Outcome::Moved(self.step())
                }
            }
            Event::SetUpLater => {
                let step = self.step();
                if !step.can_set_up_later() {
                    return Outcome::Ignored;
                }
                if !self.postponed.contains(&step) {
                    self.postponed.push(step);
                }
                self.advance()
            }
            Event::SkipSetup => {
                if self.step() == Step::Welcome {
                    self.end(Completion::Skipped)
                } else {
                    Outcome::Ignored
                }
            }
            Event::Close => self.end(if self.step() == Step::Done {
                Completion::Finished
            } else {
                Completion::Skipped
            }),
        }
    }

    fn advance(&mut self) -> Outcome {
        if self.index + 1 < self.steps.len() {
            self.index += 1;
            Outcome::Moved(self.step())
        } else {
            self.end(Completion::Finished)
        }
    }

    fn end(&mut self, completion: Completion) -> Outcome {
        self.ended = Some(completion);
        Outcome::Ended(completion)
    }
}
