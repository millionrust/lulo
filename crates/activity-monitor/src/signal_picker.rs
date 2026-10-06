//! View ▸ Send Signal to Process… (MON-10/MON-14, MON-MENU-032): the fixed
//! set of named POSIX signals the Mac's own sheet offers. Values are the
//! standard Linux signal numbers, matching `kill -l`.

/// A signal the user can choose from the Send Signal sheet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NamedSignal {
    Hup,
    Int,
    Quit,
    Abrt,
    Kill,
    Usr1,
    Usr2,
    Term,
    Cont,
    Stop,
}

impl NamedSignal {
    /// Listed in the same order the Mac's own sheet uses: the common
    /// termination signals first, then the user-defined pair, then the two
    /// job-control signals.
    pub(crate) const ALL: [Self; 10] = [
        Self::Hup,
        Self::Int,
        Self::Quit,
        Self::Abrt,
        Self::Kill,
        Self::Usr1,
        Self::Usr2,
        Self::Term,
        Self::Cont,
        Self::Stop,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Hup => "SIGHUP",
            Self::Int => "SIGINT",
            Self::Quit => "SIGQUIT",
            Self::Abrt => "SIGABRT",
            Self::Kill => "SIGKILL",
            Self::Usr1 => "SIGUSR1",
            Self::Usr2 => "SIGUSR2",
            Self::Term => "SIGTERM",
            Self::Cont => "SIGCONT",
            Self::Stop => "SIGSTOP",
        }
    }

    /// A short description of what the signal normally does, shown under
    /// its name in the sheet.
    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::Hup => "Hang up — many daemons reload their configuration",
            Self::Int => "Interrupt — the same signal Ctrl-C sends",
            Self::Quit => "Quit and dump core",
            Self::Abrt => "Abort and dump core",
            Self::Kill => "Kill immediately — cannot be caught or ignored",
            Self::Usr1 => "User-defined signal 1",
            Self::Usr2 => "User-defined signal 2",
            Self::Term => "Terminate — asks the process to exit",
            Self::Cont => "Continue a stopped process",
            Self::Stop => "Stop (pause) the process",
        }
    }

    /// The standard Linux signal number (same on every supported
    /// architecture), used both for the `pidfd_send_signal` syscall and as
    /// the `kill -s` argument for the administrator-privileged fallback.
    pub(crate) fn number(self) -> i32 {
        match self {
            Self::Hup => 1,
            Self::Int => 2,
            Self::Quit => 3,
            Self::Abrt => 6,
            Self::Kill => 9,
            Self::Usr1 => 10,
            Self::Usr2 => 12,
            Self::Term => 15,
            Self::Cont => 18,
            Self::Stop => 19,
        }
    }

    /// The `sysinfo` crate's own signal enum, used only by the non-Linux
    /// development fallback path (`process_signal::ProcessHandle::send`'s
    /// closure argument) since Linux always sends by raw number through
    /// `pidfd_send_signal` instead.
    pub(crate) fn sysinfo_signal(self) -> sysinfo::Signal {
        match self {
            Self::Hup => sysinfo::Signal::Hangup,
            Self::Int => sysinfo::Signal::Interrupt,
            Self::Quit => sysinfo::Signal::Quit,
            Self::Abrt => sysinfo::Signal::Abort,
            Self::Kill => sysinfo::Signal::Kill,
            Self::Usr1 => sysinfo::Signal::User1,
            Self::Usr2 => sysinfo::Signal::User2,
            Self::Term => sysinfo::Signal::Term,
            Self::Cont => sysinfo::Signal::Continue,
            Self::Stop => sysinfo::Signal::Stop,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_signal_has_a_distinct_label_and_number() {
        let labels: Vec<_> = NamedSignal::ALL.iter().map(|s| s.label()).collect();
        let numbers: Vec<_> = NamedSignal::ALL.iter().map(|s| s.number()).collect();
        for label in &labels {
            assert_eq!(labels.iter().filter(|other| *other == label).count(), 1);
        }
        for number in &numbers {
            assert_eq!(numbers.iter().filter(|other| *other == number).count(), 1);
        }
    }

    #[test]
    fn signal_numbers_match_standard_linux_values() {
        assert_eq!(NamedSignal::Hup.number(), 1);
        assert_eq!(NamedSignal::Term.number(), 15);
        assert_eq!(NamedSignal::Kill.number(), 9);
        assert_eq!(NamedSignal::Stop.number(), 19);
    }

    #[test]
    fn every_signal_has_a_non_empty_description() {
        for signal in NamedSignal::ALL {
            assert!(!signal.description().is_empty());
            assert!(signal.label().starts_with("SIG"));
        }
    }
}
