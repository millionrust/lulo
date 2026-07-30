//! Platform capability catalog and bounded evidence-report model.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CapabilityStatus {
    ExerciseHere,
    MissingFromStableApi,
}

impl CapabilityStatus {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::ExerciseHere => "EXERCISE",
            Self::MissingFromStableApi => "UPSTREAM SPIKE",
        }
    }

    pub(super) fn color(self) -> gpui::Hsla {
        match self {
            Self::ExerciseHere => gpui::rgb(0x248a3d).into(),
            Self::MissingFromStableApi => gpui::rgb(0xc9342f).into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Capability {
    pub(super) id: &'static str,
    pub(super) name: &'static str,
    pub(super) status: CapabilityStatus,
    pub(super) instruction: &'static str,
}

pub(super) const CAPABILITIES: &[Capability] = &[
    Capability {
        id: "window",
        name: "Native window and GPU rendering",
        status: CapabilityStatus::ExerciseHere,
        instruction: "Resize, maximize, minimize, and move this window between displays.",
    },
    Capability {
        id: "input",
        name: "Text input and IME",
        status: CapabilityStatus::ExerciseHere,
        instruction: "Enter Latin, Indic, CJK, emoji, compose, and dead-key text.",
    },
    Capability {
        id: "clipboard",
        name: "Clipboard round trip",
        status: CapabilityStatus::ExerciseHere,
        instruction: "Copy the probe, read it back, then repeat with another application.",
    },
    Capability {
        id: "file-dialog",
        name: "Native file dialog",
        status: CapabilityStatus::ExerciseHere,
        instruction: "Open the chooser, select one file, and cancel once.",
    },
    Capability {
        id: "file-drop",
        name: "External file drop",
        status: CapabilityStatus::ExerciseHere,
        instruction: "Drop one or more files on the blue target.",
    },
    Capability {
        id: "scroll-scale",
        name: "Scrolling and fractional scaling",
        status: CapabilityStatus::ExerciseHere,
        instruction: "Scroll the list and check it at 100%, 125%, 150%, and 200%.",
    },
    Capability {
        id: "display-lifecycle",
        name: "Mixed-scale display lifecycle",
        status: CapabilityStatus::ExerciseHere,
        instruction: "Move between differently scaled displays, then disconnect and reconnect one.",
    },
    Capability {
        id: "keyboard-focus",
        name: "Keyboard shortcuts and focus",
        status: CapabilityStatus::ExerciseHere,
        instruction: "Use Tab/Shift-Tab and every shortcut; verify focus remains visible.",
    },
    Capability {
        id: "suspend-resume",
        name: "Suspend and resume",
        status: CapabilityStatus::ExerciseHere,
        instruction: "Suspend with the lab open, resume, then repeat input and clipboard.",
    },
    Capability {
        id: "idle",
        name: "Idle CPU and redraw behavior",
        status: CapabilityStatus::ExerciseHere,
        instruction: "Leave the lab untouched for ten minutes and measure CPU and wakeups.",
    },
    Capability {
        id: "accessibility",
        name: "Programmatic accessibility",
        status: CapabilityStatus::MissingFromStableApi,
        instruction: "GPUI 0.2.2 exposes no accessibility tree API; test current upstream.",
    },
    Capability {
        id: "layer-shell",
        name: "Wayland layer-shell",
        status: CapabilityStatus::MissingFromStableApi,
        instruction: "GPUI 0.2.2 exposes no layer-shell API; test current upstream.",
    },
];

pub(super) const CAPABILITY_COUNT: usize = CAPABILITIES.len();
pub(super) const MAX_EVIDENCE_REPORT_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ProbeResult {
    #[default]
    Pending,
    Passed,
    Failed,
    BlockerConfirmed,
}

impl ProbeResult {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Passed => "PASS",
            Self::Failed => "FAIL",
            Self::BlockerConfirmed => "BLOCKER CONFIRMED",
        }
    }

    pub(super) fn report_value(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Passed => "pass",
            Self::Failed => "fail",
            Self::BlockerConfirmed => "blocker-confirmed",
        }
    }

    pub(super) fn color(self) -> gpui::Hsla {
        match self {
            Self::Pending => gpui::rgb(0x6e6e73).into(),
            Self::Passed => gpui::rgb(0x248a3d).into(),
            Self::Failed => gpui::rgb(0xc9342f).into(),
            Self::BlockerConfirmed => gpui::rgb(0xb35c00).into(),
        }
    }
}

pub(super) fn positive_result(capability: Capability) -> ProbeResult {
    match capability.status {
        CapabilityStatus::ExerciseHere => ProbeResult::Passed,
        CapabilityStatus::MissingFromStableApi => ProbeResult::BlockerConfirmed,
    }
}

pub(super) fn recorded_result_count(results: &[ProbeResult; CAPABILITY_COUNT]) -> usize {
    results
        .iter()
        .filter(|result| **result != ProbeResult::Pending)
        .count()
}

pub(super) fn evidence_report(results: &[ProbeResult; CAPABILITY_COUNT]) -> String {
    let recorded = recorded_result_count(results);
    let exercisable_probes_passed = CAPABILITIES
        .iter()
        .zip(results)
        .filter(|(capability, _)| capability.status == CapabilityStatus::ExerciseHere)
        .all(|(_, result)| *result == ProbeResult::Passed);
    let expected_blockers_confirmed = CAPABILITIES
        .iter()
        .zip(results)
        .filter(|(capability, _)| capability.status == CapabilityStatus::MissingFromStableApi)
        .all(|(_, result)| *result == ProbeResult::BlockerConfirmed);
    let mut report = String::with_capacity(1024);
    writeln!(report, "rmac-platform-lab-report=1").unwrap();
    writeln!(report, "os={}", std::env::consts::OS).unwrap();
    writeln!(report, "arch={}", std::env::consts::ARCH).unwrap();
    writeln!(
        report,
        "recording_complete={}",
        recorded == CAPABILITY_COUNT
    )
    .unwrap();
    writeln!(
        report,
        "exercisable_probes_passed={exercisable_probes_passed}"
    )
    .unwrap();
    writeln!(
        report,
        "expected_blockers_confirmed={expected_blockers_confirmed}"
    )
    .unwrap();
    writeln!(report, "recorded={recorded}/{CAPABILITY_COUNT}").unwrap();
    for (capability, result) in CAPABILITIES.iter().zip(results) {
        let kind = match capability.status {
            CapabilityStatus::ExerciseHere => "probe",
            CapabilityStatus::MissingFromStableApi => "blocker",
        };
        writeln!(report, "{kind}.{}={}", capability.id, result.report_value()).unwrap();
    }
    debug_assert!(report.len() <= MAX_EVIDENCE_REPORT_BYTES);
    report
}
