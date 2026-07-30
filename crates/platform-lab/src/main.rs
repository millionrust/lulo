//! Stable-GPUI platform diagnostic.
//!
//! This binary exercises the desktop contracts that must work before rmac
//! ports product applications to Linux. It intentionally stays small and does
//! not contain product behavior.

mod capabilities;
mod lab;
mod render;

use capabilities::*;
use lab::*;
use render::*;

use std::fmt::Write as _;

use gpui::{
    div, px, AppContext as _, ClipboardItem, Context, Entity, ExternalPaths,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement, PathPromptOptions, Render,
    SharedString, StatefulInteractiveElement as _, Styled, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    input::{Input, InputState},
    Sizable as _, StyledExt as _,
};
use rmac_ui::mac;

gpui::actions!(platform_lab, [CopyProbe, ReadProbe, OpenProbe]);

const CLIPBOARD_PROBE: &str = "rmac platform lab clipboard probe ✓";

fn main() {
    rmac_ui::boot("Platform Lab", 1120.0, 780.0, PlatformLab::new);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn capability_ids_are_unique() {
        let ids: HashSet<_> = CAPABILITIES
            .iter()
            .map(|capability| capability.id)
            .collect();
        assert_eq!(ids.len(), CAPABILITIES.len());
    }

    #[test]
    fn stable_api_records_phase_one_blockers() {
        for id in ["accessibility", "layer-shell"] {
            let capability = CAPABILITIES
                .iter()
                .find(|capability| capability.id == id)
                .expect("required gate exists");
            assert_eq!(capability.status, CapabilityStatus::MissingFromStableApi);
            assert_eq!(positive_result(*capability), ProbeResult::BlockerConfirmed);
        }
    }

    #[test]
    fn core_desktop_probes_are_exercisable() {
        for id in [
            "window",
            "input",
            "clipboard",
            "file-dialog",
            "file-drop",
            "scroll-scale",
            "display-lifecycle",
            "keyboard-focus",
            "suspend-resume",
            "idle",
        ] {
            let capability = CAPABILITIES
                .iter()
                .find(|capability| capability.id == id)
                .expect("core probe exists");
            assert_eq!(capability.status, CapabilityStatus::ExerciseHere);
        }
    }

    #[test]
    fn evidence_summaries_never_include_clipboard_or_path_content() {
        assert_eq!(
            ClipboardProbeResult::ExactProbe.summary(),
            "Built-in clipboard probe matched exactly"
        );
        assert_eq!(
            ClipboardProbeResult::OtherText.summary(),
            "Clipboard text received; content hidden"
        );
        assert_eq!(
            selected_file_summary(true),
            "One file selected; path hidden"
        );
        assert_eq!(
            dropped_paths_summary(2),
            "2 external paths received; paths hidden"
        );
    }

    #[test]
    fn evidence_report_is_bounded_ordered_and_pending_by_default() {
        let report = evidence_report(&[ProbeResult::Pending; CAPABILITY_COUNT]);

        assert!(report.starts_with("rmac-platform-lab-report=1\n"));
        assert!(report.contains("recording_complete=false\n"));
        assert!(report.contains("exercisable_probes_passed=false\n"));
        assert!(report.contains("expected_blockers_confirmed=false\n"));
        assert!(report.contains(&format!("recorded=0/{CAPABILITY_COUNT}\n")));
        assert!(report.contains("probe.input=pending\n"));
        assert!(report.contains("blocker.accessibility=pending\n"));
        assert!(report.len() <= MAX_EVIDENCE_REPORT_BYTES);
        for private in ["/home/", "/Users/", "clipboard probe ✓"] {
            assert!(!report.contains(private));
        }
    }

    #[test]
    fn complete_report_distinguishes_pass_fail_and_confirmed_blockers() {
        let mut results = [ProbeResult::Passed; CAPABILITY_COUNT];
        let accessibility = CAPABILITIES
            .iter()
            .position(|capability| capability.id == "accessibility")
            .unwrap();
        let layer_shell = CAPABILITIES
            .iter()
            .position(|capability| capability.id == "layer-shell")
            .unwrap();
        let clipboard = CAPABILITIES
            .iter()
            .position(|capability| capability.id == "clipboard")
            .unwrap();
        results[accessibility] = ProbeResult::BlockerConfirmed;
        results[layer_shell] = ProbeResult::BlockerConfirmed;

        let passing_report = evidence_report(&results);
        assert!(passing_report.contains("recording_complete=true\n"));
        assert!(passing_report.contains("exercisable_probes_passed=true\n"));
        assert!(passing_report.contains("expected_blockers_confirmed=true\n"));

        results[clipboard] = ProbeResult::Failed;
        let failed_report = evidence_report(&results);
        assert!(failed_report.contains("recording_complete=true\n"));
        assert!(failed_report.contains("exercisable_probes_passed=false\n"));
        assert!(failed_report.contains("expected_blockers_confirmed=true\n"));
        assert!(
            failed_report.contains(&format!("recorded={CAPABILITY_COUNT}/{CAPABILITY_COUNT}\n"))
        );
        assert!(failed_report.contains("probe.clipboard=fail\n"));
        assert!(failed_report.contains("blocker.accessibility=blocker-confirmed\n"));
        assert!(failed_report.contains("blocker.layer-shell=blocker-confirmed\n"));
    }
}
